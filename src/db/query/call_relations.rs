use std::collections::{HashMap, HashSet};

use rusqlite::{params, params_from_iter, ToSql};

use super::call_resolution::CallTargetResolver;
use super::DbQueryContext;
use crate::models::{CalleeInfo, CallerInfo, FunctionInfo, FunctionKey, Location};
use crate::utils::{sort_callees_by_file_line, sort_callers_by_file_line, split_function_target};

#[derive(Debug)]
pub(super) struct CallLookupRow {
    file_id: i64,
    file: String,
    caller: Option<String>,
    caller_class_name: Option<String>,
    object_name: Option<String>,
    line: usize,
}

#[derive(Debug)]
pub(super) struct CalleeLookupRow {
    file_id: i64,
    file: String,
    callee: String,
    object_name: Option<String>,
    caller_class_name: Option<String>,
    line: usize,
}

#[derive(Debug, Clone)]
pub(super) struct DbFunctionNode {
    pub(super) function_id: i64,
    pub(super) file_id: i64,
    pub(super) function: FunctionInfo,
}

impl DbFunctionNode {
    pub(super) fn key(&self) -> FunctionKey {
        FunctionKey::from(&self.function)
    }
}

pub(crate) struct CallRelationQuery<'a> {
    ctx: DbQueryContext<'a>,
}

impl<'a> CallRelationQuery<'a> {
    pub(crate) fn new(ctx: DbQueryContext<'a>) -> Self {
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
            SELECT c.file_id, f.path, c.caller, c.caller_class_name, c.object_name, c.start_line
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
            Ok(CallLookupRow {
                file_id: row.get(0)?,
                file: row.get(1)?,
                caller: row.get(2)?,
                caller_class_name: row.get(3)?,
                object_name: row.get(4)?,
                line: row.get::<_, i64>(5)? as usize,
            })
        })?;

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        let mut field_type_cache = HashMap::new();
        let mut param_type_cache = HashMap::new();
        let mut node_cache = HashMap::new();
        for row in rows {
            let row = row?;
            if let Some(target_object) = target_object {
                if row.object_name.as_deref() != Some(target_object) {
                    continue;
                }
            }
            if let Some(class_name) = class_name {
                let Some(caller_name) = row.caller.as_deref() else {
                    continue;
                };
                let Some(caller) = self.resolve_enclosing_db_function(
                    row.file_id,
                    &row.file,
                    caller_name,
                    row.caller_class_name.as_deref(),
                    row.line,
                    &mut node_cache,
                )?
                else {
                    continue;
                };
                if !resolver.matches_call_target_for_caller(
                    &caller,
                    row.object_name.as_deref(),
                    class_name,
                    &mut field_type_cache,
                    &mut param_type_cache,
                )? {
                    continue;
                }
            }
            let caller = row.caller.unwrap_or_else(|| "<module>".to_string());
            let key = (row.file.clone(), caller.clone(), row.line);
            if seen.insert(key) {
                results.push(CallerInfo {
                    caller,
                    line: row.line,
                    file: row.file,
                });
            }
        }

        if resolver.is_python_property(function_name, class_name)? {
            let mut property_sql = String::from(
                "
                SELECT ppc.file_id, f.path, ppc.caller, ppc.caller_class_name, ppc.object_name, ppc.line
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
            property_sql.push_str(" ORDER BY f.path, ppc.line");
            let mut property_stmt = self.ctx.conn.prepare(&property_sql)?;
            let property_rows =
                property_stmt.query_map(params_from_iter(property_params), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)? as usize,
                    ))
                })?;
            for row in property_rows {
                let (file_id, file, caller_name, caller_class_name, object_name, line) = row?;
                if let Some(class_name) = class_name {
                    let caller = self.resolve_enclosing_db_function(
                        file_id,
                        &file,
                        &caller_name,
                        caller_class_name.as_deref(),
                        line,
                        &mut node_cache,
                    )?;
                    if !resolver.matches_property_target_for_caller(
                        caller.as_ref(),
                        object_name.as_deref(),
                        class_name,
                        &mut field_type_cache,
                        &mut param_type_cache,
                    )? {
                        continue;
                    }
                }
                let key = (file.clone(), caller_name.clone(), line);
                if seen.insert(key) {
                    results.push(CallerInfo {
                        caller: caller_name,
                        line,
                        file,
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
        let relevant_files = self.relevant_function_file_ids(function_name, class_name)?;

        let mut sql = String::from(
            "
            SELECT c.file_id, f.path, c.callee, c.object_name, c.caller_class_name, c.start_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.caller = ?1
            ",
        );
        let language = self.ctx.language;
        if class_name.is_some() {
            sql.push_str(" AND c.caller_class_name = ?2");
        }
        if language.is_some() {
            sql.push_str(if class_name.is_some() {
                " AND f.language = ?3"
            } else {
                " AND f.language = ?2"
            });
        }
        sql.push_str(" ORDER BY f.path, c.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows: Vec<CalleeLookupRow> = match (class_name, language) {
            (Some(class_name), Some(language)) => stmt
                .query_map(params![function_name, class_name, language], |row| {
                    Ok(CalleeLookupRow {
                        file_id: row.get(0)?,
                        file: row.get(1)?,
                        callee: row.get(2)?,
                        object_name: row.get(3)?,
                        caller_class_name: row.get(4)?,
                        line: row.get::<_, i64>(5)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(class_name), None) => stmt
                .query_map(params![function_name, class_name], |row| {
                    Ok(CalleeLookupRow {
                        file_id: row.get(0)?,
                        file: row.get(1)?,
                        callee: row.get(2)?,
                        object_name: row.get(3)?,
                        caller_class_name: row.get(4)?,
                        line: row.get::<_, i64>(5)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(language)) => stmt
                .query_map(params![function_name, language], |row| {
                    Ok(CalleeLookupRow {
                        file_id: row.get(0)?,
                        file: row.get(1)?,
                        callee: row.get(2)?,
                        object_name: row.get(3)?,
                        caller_class_name: row.get(4)?,
                        line: row.get::<_, i64>(5)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([function_name], |row| {
                    Ok(CalleeLookupRow {
                        file_id: row.get(0)?,
                        file: row.get(1)?,
                        callee: row.get(2)?,
                        object_name: row.get(3)?,
                        caller_class_name: row.get(4)?,
                        line: row.get::<_, i64>(5)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
        };

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            if !relevant_files.is_empty() && !relevant_files.contains(&row.file_id) {
                continue;
            }
            let mut callee_name = row.callee.clone();
            if let Some(object_name) = &row.object_name {
                callee_name = format!("{}.{}", object_name, callee_name);
            }
            let key = (
                row.file.clone(),
                callee_name.clone(),
                row.caller_class_name.clone(),
            );
            if seen.insert(key) {
                results.push(CalleeInfo {
                    callee: callee_name,
                    line: row.line,
                    file: row.file,
                    class_name: row.caller_class_name,
                });
            }
        }
        sort_callees_by_file_line(&mut results);
        Ok(results)
    }

    pub(super) fn find_exact_function_nodes(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<DbFunctionNode>> {
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
        let rows: Vec<DbFunctionNode> = match (class_name, language) {
            (Some(class_name), Some(language)) => stmt
                .query_map(params![function_name, class_name, language], |row| {
                    Self::row_to_db_function_node(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(class_name), None) => stmt
                .query_map(params![function_name, class_name], |row| {
                    Self::row_to_db_function_node(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(language)) => stmt
                .query_map(params![function_name, language], |row| {
                    Self::row_to_db_function_node(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([function_name], Self::row_to_db_function_node)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    pub(super) fn load_function_nodes_by_name(
        &self,
        function_name: &str,
        cache: &mut HashMap<(String, Option<String>), Vec<DbFunctionNode>>,
    ) -> anyhow::Result<Vec<DbFunctionNode>> {
        let key = (function_name.to_string(), None);
        if let Some(cached) = cache.get(&key) {
            return Ok(cached.clone());
        }

        let loaded = self.find_exact_function_nodes(function_name, None)?;
        cache.insert(key, loaded.clone());
        Ok(loaded)
    }

    pub(super) fn resolve_enclosing_db_function(
        &self,
        file_id: i64,
        file: &str,
        function_name: &str,
        class_name: Option<&str>,
        line: usize,
        cache: &mut HashMap<(String, Option<String>), Vec<DbFunctionNode>>,
    ) -> anyhow::Result<Option<DbFunctionNode>> {
        let cache_key = (function_name.to_string(), class_name.map(str::to_string));
        let candidates = if let Some(cached) = cache.get(&cache_key) {
            cached.clone()
        } else {
            let loaded = self.find_exact_function_nodes(function_name, class_name)?;
            cache.insert(cache_key.clone(), loaded.clone());
            loaded
        };

        Ok(candidates.into_iter().find(|candidate| {
            candidate.file_id == file_id
                && candidate.function.location.file == file
                && candidate.function.location.start_line <= line
                && line <= candidate.function.location.end_line
        }))
    }

    fn relevant_function_file_ids(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<HashSet<i64>> {
        let mut sql = String::from(
            "
            SELECT DISTINCT fn.file_id, f.language
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
        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows: Vec<i64> = match (class_name, language) {
            (Some(class_name), Some(language)) => stmt
                .query_map(params![function_name, class_name, language], |row| {
                    row.get::<_, i64>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(class_name), None) => stmt
                .query_map(params![function_name, class_name], |row| {
                    row.get::<_, i64>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(language)) => stmt
                .query_map(params![function_name, language], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([function_name], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows.into_iter().collect())
    }

    fn row_to_db_function_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbFunctionNode> {
        Ok(DbFunctionNode {
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
