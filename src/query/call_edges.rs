use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, ToSql};

use super::call_resolver::CallTargetResolver;
use super::QueryContext;
use crate::models::{CalleeInfo, CallerInfo, FunctionInfo, FunctionKey, Location};
use crate::parser::call_targets::{has_non_self_object_target, split_function_target};
use crate::utils::{
    select_most_specific_by_line, sort_callees_by_file_line, sort_callers_by_file_line,
};

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
    location: Location,
    callee: String,
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

    pub(crate) fn find_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CallerInfo>> {
        let resolver = CallTargetResolver::new(self.ctx);
        let (target_function, target_object) = split_function_target(function_name);
        let mut sql = String::from(
            "
            SELECT c.file_id, f.path, c.caller, c.caller_class_name, c.object_name, c.start_line, c.end_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.callee = ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&target_function];
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, c.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(CallerRow {
                file_id: row.get(0)?,
                file: row.get(1)?,
                caller: row.get(2)?,
                caller_class_name: row.get(3)?,
                object_name: row.get(4)?,
                start_line: row.get::<_, i64>(5)? as usize,
                end_line: row.get::<_, i64>(6)? as usize,
            })
        })?;

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        let mut field_type_cache = HashMap::new();
        let mut param_type_cache = HashMap::new();
        let mut node_cache = HashMap::new();
        let mut resolution_cache = HashMap::new();
        let mut enclosing_caches = EnclosingFunctionCaches {
            candidates: &mut node_cache,
            resolutions: &mut resolution_cache,
        };
        let unique_method_target = match class_name {
            Some(class_name) => resolver.has_unique_method_target(target_function, class_name)?,
            None => false,
        };
        for row in rows {
            let row = row?;
            if let Some(target_object) = target_object {
                if row.object_name.as_deref() != Some(target_object) {
                    continue;
                }
            }
            if let Some(class_name) = class_name {
                if let Some(caller_name) = row.caller.as_deref() {
                    let Some(caller) = self.resolve_enclosing_function(
                        row.file_id,
                        &row.file,
                        caller_name,
                        row.caller_class_name.as_deref(),
                        row.start_line,
                        &mut enclosing_caches,
                    )?
                    else {
                        continue;
                    };
                    if !(resolver.matches_call_target(
                        &caller,
                        row.object_name.as_deref(),
                        class_name,
                        &mut field_type_cache,
                        &mut param_type_cache,
                    )? || unique_method_target
                        && has_non_self_object_target(row.object_name.as_deref()))
                    {
                        continue;
                    }
                } else if !(resolver.matches_call_target_without_enclosing_function(
                    row.caller_class_name.as_deref(),
                    row.object_name.as_deref(),
                    class_name,
                ) || unique_method_target
                    && has_non_self_object_target(row.object_name.as_deref()))
                {
                    continue;
                }
            }
            let caller = row.caller.unwrap_or_else(|| "<module>".to_string());
            let key = (
                row.file.clone(),
                caller.clone(),
                row.start_line,
                row.end_line,
            );
            if seen.insert(key) {
                results.push(CallerInfo {
                    caller,
                    location: Location {
                        file: row.file,
                        start_line: row.start_line,
                        end_line: row.end_line,
                    },
                });
            }
        }

        if resolver.is_python_property(function_name, class_name)? {
            let mut property_sql = String::from(
                "
                SELECT ppc.file_id, f.path, ppc.caller, ppc.caller_class_name, ppc.object_name, ppc.object_type, ppc.start_line, ppc.end_line
                FROM python_property_callers ppc
                JOIN files f ON f.id = ppc.file_id
                WHERE ppc.property_name = ?1
                ",
            );
            let mut property_params: Vec<&dyn ToSql> = vec![&target_function];
            if let Some(language) = self.ctx.language.as_ref() {
                property_sql.push_str(" AND f.language = ?2");
                property_params.push(language);
            }
            property_sql.push_str(" ORDER BY f.path, ppc.start_line");
            let mut property_stmt = self.ctx.conn.prepare(&property_sql)?;
            let property_rows =
                property_stmt.query_map(params_from_iter(property_params), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, i64>(6)? as usize,
                        row.get::<_, i64>(7)? as usize,
                    ))
                })?;
            for row in property_rows {
                let (
                    file_id,
                    file,
                    caller_name,
                    caller_class_name,
                    object_name,
                    object_type,
                    start_line,
                    end_line,
                ) = row?;
                if let Some(class_name) = class_name {
                    if caller_name == "<module>" {
                        if !resolver.matches_property_target_without_enclosing_function(
                            object_name.as_deref(),
                            object_type.as_deref(),
                            class_name,
                        ) {
                            continue;
                        }
                    } else {
                        let caller = self.resolve_enclosing_function(
                            file_id,
                            &file,
                            &caller_name,
                            caller_class_name.as_deref(),
                            start_line,
                            &mut enclosing_caches,
                        )?;
                        if !resolver.matches_property_target(
                            caller.as_ref(),
                            object_name.as_deref(),
                            class_name,
                            &mut field_type_cache,
                            &mut param_type_cache,
                        )? {
                            continue;
                        }
                    }
                }
                let key = (file.clone(), caller_name.clone(), start_line, end_line);
                if seen.insert(key) {
                    results.push(CallerInfo {
                        caller: caller_name,
                        location: Location {
                            file,
                            start_line,
                            end_line,
                        },
                    });
                }
            }
        }

        sort_callers_by_file_line(&mut results);
        Ok(results)
    }

    pub(crate) fn find_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CalleeInfo>> {
        let mut sql = String::from(
            "
            SELECT c.file_id, f.path, c.callee, c.object_name, c.caller_class_name, c.start_line, c.end_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.caller = ?1
            ",
        );
        let mut params = vec![Value::from(function_name.to_string())];
        let mut next_param_index = 2;
        let language = self.ctx.language;
        if let Some(class_name) = class_name {
            sql.push_str(&format!(" AND c.caller_class_name = ?{next_param_index}"));
            params.push(Value::from(class_name.to_string()));
            next_param_index += 1;
        }
        if let Some(language) = language {
            sql.push_str(&format!(" AND f.language = ?{next_param_index}"));
            params.push(Value::from(language.to_string()));
            next_param_index += 1;
        }
        sql.push_str(&format!(
            "
            AND EXISTS (
                SELECT 1
                FROM functions fn
                JOIN files ff ON ff.id = fn.file_id
                WHERE fn.file_id = c.file_id
                  AND fn.name = ?{next_param_index}
            "
        ));
        params.push(Value::from(function_name.to_string()));
        next_param_index += 1;
        if let Some(class_name) = class_name {
            sql.push_str(&format!(" AND fn.class_name = ?{next_param_index}"));
            params.push(Value::from(class_name.to_string()));
            next_param_index += 1;
        }
        if let Some(language) = language {
            sql.push_str(&format!(" AND ff.language = ?{next_param_index}"));
            params.push(Value::from(language.to_string()));
        }
        sql.push_str(" ) ORDER BY f.path, c.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows: Vec<CalleeRow> = stmt
            .query_map(params_from_iter(params.iter()), |row| {
                Ok(CalleeRow {
                    callee: {
                        let callee: String = row.get(2)?;
                        let object_name: Option<String> = row.get(3)?;
                        match object_name {
                            Some(object_name) => format!("{}.{}", object_name, callee),
                            None => callee,
                        }
                    },
                    location: Location {
                        file: row.get(1)?,
                        start_line: row.get::<_, i64>(5)? as usize,
                        end_line: row.get::<_, i64>(6)? as usize,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            let key = (
                row.location.file.clone(),
                row.callee.clone(),
                row.location.start_line,
                row.location.end_line,
            );
            if seen.insert(key) {
                results.push(CalleeInfo {
                    callee: row.callee,
                    location: row.location,
                });
            }
        }

        let mut property_sql = String::from(
            "
            SELECT f.path, ppc.property_name, ppc.object_name, ppc.start_line, ppc.end_line
            FROM python_property_callers ppc
            JOIN files f ON f.id = ppc.file_id
            WHERE ppc.caller = ?1
            ",
        );
        let mut property_params = vec![Value::from(function_name.to_string())];
        let mut next_property_param_index = 2;
        if let Some(class_name) = class_name {
            property_sql.push_str(&format!(
                " AND ppc.caller_class_name = ?{next_property_param_index}"
            ));
            property_params.push(Value::from(class_name.to_string()));
            next_property_param_index += 1;
        } else {
            property_sql.push_str(" AND ppc.caller_class_name IS NULL");
        }
        if let Some(language) = self.ctx.language.as_ref() {
            property_sql.push_str(&format!(" AND f.language = ?{next_property_param_index}"));
            property_params.push(Value::from(language.to_string()));
            next_property_param_index += 1;
        }
        property_sql.push_str(&format!(
            "
            AND EXISTS (
                SELECT 1
                FROM functions fn
                JOIN files ff ON ff.id = fn.file_id
                WHERE fn.file_id = ppc.file_id
                  AND fn.name = ?{next_property_param_index}
            "
        ));
        property_params.push(Value::from(function_name.to_string()));
        next_property_param_index += 1;
        if let Some(class_name) = class_name {
            property_sql.push_str(&format!(
                " AND fn.class_name = ?{next_property_param_index}"
            ));
            property_params.push(Value::from(class_name.to_string()));
            next_property_param_index += 1;
        }
        if let Some(language) = self.ctx.language.as_ref() {
            property_sql.push_str(&format!(" AND ff.language = ?{next_property_param_index}"));
            property_params.push(Value::from(language.to_string()));
        }
        property_sql.push_str(" )");
        property_sql.push_str(
            "
            ORDER BY f.path, ppc.start_line
            ",
        );

        let mut property_stmt = self.ctx.conn.prepare(&property_sql)?;
        let property_rows =
            property_stmt.query_map(params_from_iter(property_params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)? as usize,
                    row.get::<_, i64>(4)? as usize,
                ))
            })?;
        for row in property_rows {
            let (file, property_name, object_name, start_line, end_line) = row?;
            let callee = match object_name {
                Some(object_name) => format!("{}.{}", object_name, property_name),
                None => property_name,
            };
            let key = (file.clone(), callee.clone(), start_line, end_line);
            if seen.insert(key) {
                results.push(CalleeInfo {
                    callee,
                    location: Location {
                        file,
                        start_line,
                        end_line,
                    },
                });
            }
        }
        sort_callees_by_file_line(&mut results);
        Ok(results)
    }

    pub(super) fn load_exact_functions(
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

        let loaded = self.load_exact_functions_shared(function_name, None)?;
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
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
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
            let loaded = self.load_exact_functions_shared(function_name, class_name)?;
            caches.candidates.insert(cache_key, Arc::clone(&loaded));
            loaded
        };

        let resolved =
            resolve_enclosing_function_from_candidates(candidates.as_ref(), file_id, file, line);
        caches.resolutions.insert(resolution_key, resolved.clone());
        Ok(resolved)
    }

    fn load_exact_functions_shared(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<IndexedFunctionSlice> {
        Ok(self.load_exact_functions(function_name, class_name)?.into())
    }

    fn function_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexedFunction> {
        Ok(IndexedFunction {
            function_id: row.get(0)?,
            file_id: row.get(1)?,
            function: FunctionInfo {
                name: row.get(3)?,
                location: Location {
                    file: row.get(2)?,
                    start_line: row.get::<_, i64>(5)? as usize,
                    end_line: row.get::<_, i64>(6)? as usize,
                },
                body: String::new(),
                class_name: row.get(4)?,
                params: Vec::new(),
            },
        })
    }
}

fn resolve_enclosing_function_from_candidates(
    candidates: &[IndexedFunction],
    file_id: i64,
    file: &str,
    line: usize,
) -> Option<IndexedFunction> {
    let file_start =
        candidates.partition_point(|candidate| candidate.function.location.file.as_str() < file);
    let file_end =
        candidates.partition_point(|candidate| candidate.function.location.file.as_str() <= file);
    let file_candidates = &candidates[file_start..file_end];

    select_most_specific_by_line(file_candidates, line, |candidate| {
        (
            candidate.function.location.start_line,
            candidate.function.location.end_line,
        )
    })
    .filter(|candidate| candidate.file_id == file_id)
    .cloned()
}
