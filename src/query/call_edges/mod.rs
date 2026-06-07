use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;
use std::sync::Arc;

use rusqlite::{params, params_from_iter, ToSql};

use super::QueryContext;
use crate::models::{FunctionInfo, FunctionKey, Location};
use crate::utils::select_most_specific_by_line;

mod callees;
mod callers;

#[derive(Debug)]
pub(super) struct CallerRow {
    file_id: i64,
    file: String,
    caller: Option<String>,
    caller_class_name: Option<String>,
    object_name: Option<String>,
    start_line: usize,
    end_line: usize,
}

#[derive(Debug)]
pub(super) struct CalleeRow {
    file_id: i64,
    location: Location,
    callee: String,
    object_name: Option<String>,
    caller_class_name: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct IndexedFunction {
    pub(super) function_id: i64,
    pub(super) file_id: i64,
    pub(super) function: FunctionInfo,
}

impl IndexedFunction {
    pub(super) fn key(&self) -> FunctionKey {
        FunctionKey::from(&self.function)
    }
}

pub(crate) struct CallEdgeQuery<'a> {
    ctx: QueryContext<'a>,
}

type IndexedFunctionCacheKey = (String, Option<String>);
type IndexedResolutionCacheKey = (i64, String, String, Option<String>, usize);
type IndexedFunctionSlice = Arc<[IndexedFunction]>;
type IndexedFunctionCache = HashMap<IndexedFunctionCacheKey, IndexedFunctionSlice>;
type IndexedResolutionCache = HashMap<IndexedResolutionCacheKey, Option<IndexedFunction>>;

pub(super) struct EnclosingFunctionCaches<'a> {
    pub(super) candidates: &'a mut IndexedFunctionCache,
    pub(super) resolutions: &'a mut IndexedResolutionCache,
}

impl<'a> CallEdgeQuery<'a> {
    pub(crate) fn new(ctx: QueryContext<'a>) -> Self {
        Self { ctx }
    }

    pub(super) fn load_functions_by_name_class(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<IndexedFunction>> {
        let mut sql = String::from(
            "
            SELECT fn.id, fn.file_id, f.path, fn.name, fn.class_name, fn.start_line, fn.end_line
            FROM functions fn
            JOIN files f ON f.id = fn.file_id
            WHERE fn.name = ?1
            ",
        );
        let language = self.ctx.language;
        if class_name.is_some() {
            sql.push_str(" AND fn.class_name = ?2");
        }
        if language.is_some() {
            sql.push_str(if class_name.is_some() {
                " AND f.language = ?3"
            } else {
                " AND f.language = ?2"
            });
        }
        sql.push_str(" ORDER BY f.path, fn.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows: Vec<IndexedFunction> = match (class_name, language) {
            (Some(class_name), Some(language)) => stmt
                .query_map(params![function_name, class_name, language], |row| {
                    Self::function_from_row(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(class_name), None) => stmt
                .query_map(params![function_name, class_name], |row| {
                    Self::function_from_row(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(language)) => stmt
                .query_map(params![function_name, language], |row| {
                    Self::function_from_row(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([function_name], Self::function_from_row)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    pub(super) fn load_functions_by_name(
        &self,
        function_name: &str,
        cache: &mut IndexedFunctionCache,
    ) -> anyhow::Result<IndexedFunctionSlice> {
        let key = (function_name.to_string(), None);
        if let Some(cached) = cache.get(&key) {
            return Ok(Arc::clone(cached));
        }

        let loaded: IndexedFunctionSlice = self.load_functions_by_name_class(function_name, None)?.into();
        cache.insert(key, Arc::clone(&loaded));
        Ok(loaded)
    }

    pub(super) fn load_functions_by_names(
        &self,
        function_names: &BTreeSet<String>,
        cache: &mut IndexedFunctionCache,
    ) -> anyhow::Result<()> {
        let missing_names: Vec<String> = function_names
            .iter()
            .filter(|function_name| !cache.contains_key(&((*function_name).clone(), None)))
            .cloned()
            .collect();
        if missing_names.is_empty() {
            return Ok(());
        }

        let placeholders = std::iter::repeat_n("?", missing_names.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut sql = format!(
            "
            SELECT fn.id, fn.file_id, f.path, fn.name, fn.class_name, fn.start_line, fn.end_line
            FROM functions fn
            JOIN files f ON f.id = fn.file_id
            WHERE fn.name IN ({placeholders})
            "
        );

        let mut params: Vec<&dyn ToSql> = missing_names
            .iter()
            .map(|name| name as &dyn ToSql)
            .collect();
        if let Some(language) = self.ctx.language.as_ref() {
            let _ = write!(sql, " AND f.language = ?{}", params.len() + 1);
            params.push(language);
        }
        sql.push_str(" ORDER BY fn.name, f.path, fn.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), Self::function_from_row)?;

        let mut grouped: HashMap<String, Vec<IndexedFunction>> = HashMap::new();
        for row in rows {
            let function = row?;
            grouped
                .entry(function.function.name.clone())
                .or_default()
                .push(function);
        }

        for function_name in missing_names {
            let functions = grouped.remove(&function_name).unwrap_or_default();
            cache.insert((function_name, None), functions.into());
        }
        Ok(())
    }

    pub(super) fn resolve_enclosing_function(
        &self,
        file_id: i64,
        file: &str,
        function_name: &str,
        class_name: Option<&str>,
        line: usize,
        caches: &mut EnclosingFunctionCaches<'_>,
    ) -> anyhow::Result<Option<IndexedFunction>> {
        let resolution_key = (
            file_id,
            file.to_string(),
            function_name.to_string(),
            class_name.map(str::to_string),
            line,
        );
        if let Some(cached) = caches.resolutions.get(&resolution_key) {
            return Ok(cached.clone());
        }

        let cache_key = (function_name.to_string(), class_name.map(str::to_string));
        let candidates = if let Some(cached) = caches.candidates.get(&cache_key) {
            Arc::clone(cached)
        } else {
            let loaded: IndexedFunctionSlice =
                self.load_functions_by_name_class(function_name, class_name)?.into();
            caches.candidates.insert(cache_key, Arc::clone(&loaded));
            loaded
        };

        let file_start = candidates
            .partition_point(|candidate| candidate.function.location.file.as_str() < file);
        let file_end = candidates
            .partition_point(|candidate| candidate.function.location.file.as_str() <= file);
        let file_candidates = &candidates[file_start..file_end];

        let resolved = select_most_specific_by_line(file_candidates, line, |candidate| {
            (
                candidate.function.location.start_line,
                candidate.function.location.end_line,
            )
        })
        .filter(|candidate| candidate.file_id == file_id)
        .cloned();
        caches.resolutions.insert(resolution_key, resolved.clone());
        Ok(resolved)
    }

    fn function_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexedFunction> {
        Ok(IndexedFunction {
            function_id: row.get(0)?,
            file_id: row.get(1)?,
            function: FunctionInfo {
                name: row.get(3)?,
                location: Location {
                    file: row.get(2)?,
                    start_line: row.get::<_, usize>(5)?,
                    end_line: row.get::<_, usize>(6)?,
                },
                body: String::new(),
                class_name: row.get(4)?,
                params: Vec::new(),
            },
        })
    }
}
