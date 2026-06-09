use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use rusqlite::types::Value;
use rusqlite::params_from_iter;

use super::super::call_resolver::CallTargetResolver;
use super::{CalleeRow, CallEdgeQuery, EnclosingFunctionCaches};
use crate::models::{CalleeInfo, Location};

impl CallEdgeQuery<'_> {
    pub(crate) fn find_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CalleeInfo>> {
        let mut callees = self.collect_callees(function_name, class_name)?;
        callees.append(&mut self.collect_property_callees(function_name, class_name)?);
        let mut seen = HashSet::new();
        callees.retain(|callee| {
            seen.insert((
                callee.location.file.clone(),
                callee.callee.clone(),
                callee.location.start_line,
                callee.location.end_line,
            ))
        });
        callees.sort_by(|left, right| {
            left.location
                .file
                .cmp(&right.location.file)
                .then(left.location.start_line.cmp(&right.location.start_line))
                .then(left.callee.cmp(&right.callee))
        });
        Ok(callees)
    }

    fn collect_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CalleeInfo>> {
        let (sql, params) = self.build_callee_sql(function_name, class_name);
        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let resolver = CallTargetResolver::new(self.ctx);
        let mut field_type_cache = HashMap::new();
        let mut param_type_cache = HashMap::new();
        let mut node_cache = HashMap::new();
        let mut resolution_cache = HashMap::new();
        let mut enclosing_caches = EnclosingFunctionCaches {
            candidates: &mut node_cache,
            resolutions: &mut resolution_cache,
        };
        let rows: Vec<CalleeRow> = stmt
            .query_map(params_from_iter(params.iter()), |row| {
                Ok(CalleeRow {
                    file_id: row.get(0)?,
                    callee: row.get(2)?,
                    object_name: row.get(3)?,
                    caller_class_name: row.get(4)?,
                    location: Location {
                        file: row.get(1)?,
                        start_line: row.get::<_, usize>(5)?,
                        end_line: row.get::<_, usize>(6)?,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut callees = Vec::new();
        for row in rows {
            if row.object_name.as_deref() == Some("super()") {
                if let Some(caller) = self.resolve_enclosing_function(
                    row.file_id,
                    &row.location.file,
                    function_name,
                    row.caller_class_name.as_deref(),
                    row.location.start_line,
                    &mut enclosing_caches,
                )? {
                    let candidates =
                        self.load_functions_by_name(&row.callee, enclosing_caches.candidates)?;
                    let resolved_targets = resolver.resolve_forward_targets(
                        &caller,
                        row.object_name.as_deref(),
                        candidates.as_ref(),
                        &mut field_type_cache,
                        &mut param_type_cache,
                    )?;
                    if !resolved_targets.is_empty() {
                        for candidate in resolved_targets {
                            let callee = match candidate.function.class_name.as_deref() {
                                Some(class_name) => {
                                    format!("{}.{}", class_name, candidate.function.name)
                                }
                                None => candidate.function.name.clone(),
                            };
                            callees.push(CalleeInfo {
                                callee,
                                location: row.location.clone(),
                            });
                        }
                        continue;
                    }
                }
            }
            let callee = match row.object_name {
                Some(object_name) => format!("{}.{}", object_name, row.callee),
                None => row.callee,
            };
            callees.push(CalleeInfo {
                callee,
                location: row.location,
            });
        }

        Ok(callees)
    }

    fn collect_property_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CalleeInfo>> {
        let (property_sql, property_params) =
            self.build_property_callee_sql(function_name, class_name);
        let mut property_stmt = self.ctx.conn.prepare(&property_sql)?;
        let property_rows =
            property_stmt.query_map(params_from_iter(property_params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, usize>(3)?,
                    row.get::<_, usize>(4)?,
                ))
            })?;
        let mut callees = Vec::new();
        for row in property_rows {
            let (file, property_name, object_name, start_line, end_line) = row?;
            let callee = match object_name {
                Some(object_name) => format!("{object_name}.{property_name}"),
                None => property_name,
            };
            callees.push(CalleeInfo {
                callee,
                location: Location {
                    file,
                    start_line,
                    end_line,
                },
            });
        }

        Ok(callees)
    }

    fn build_callee_sql(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> (String, Vec<Value>) {
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
            let _ = write!(sql, " AND c.caller_class_name = ?{next_param_index}");
            params.push(Value::from(class_name.to_string()));
            next_param_index += 1;
        }
        if let Some(language) = language {
            let _ = write!(sql, " AND f.language = ?{next_param_index}");
            params.push(Value::from(language.to_string()));
            next_param_index += 1;
        }
        let _ = write!(
            sql,
            "
            AND EXISTS (
                SELECT 1
                FROM functions fn
                JOIN files ff ON ff.id = fn.file_id
                WHERE fn.file_id = c.file_id
                  AND fn.name = ?{next_param_index}
            "
        );
        params.push(Value::from(function_name.to_string()));
        next_param_index += 1;
        if let Some(class_name) = class_name {
            let _ = write!(sql, " AND fn.class_name = ?{next_param_index}");
            params.push(Value::from(class_name.to_string()));
            next_param_index += 1;
        }
        if let Some(language) = language {
            let _ = write!(sql, " AND ff.language = ?{next_param_index}");
            params.push(Value::from(language.to_string()));
        }
        sql.push_str(" ) ORDER BY f.path, c.start_line");
        (sql, params)
    }

    fn build_property_callee_sql(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> (String, Vec<Value>) {
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
            let _ = write!(
                property_sql,
                " AND ppc.caller_class_name = ?{next_property_param_index}"
            );
            property_params.push(Value::from(class_name.to_string()));
            next_property_param_index += 1;
        } else {
            property_sql.push_str(" AND ppc.caller_class_name IS NULL");
        }
        if let Some(language) = self.ctx.language.as_ref() {
            let _ = write!(
                property_sql,
                " AND f.language = ?{next_property_param_index}"
            );
            property_params.push(Value::from(language.to_string()));
            next_property_param_index += 1;
        }
        let _ = write!(
            property_sql,
            "
            AND EXISTS (
                SELECT 1
                FROM functions fn
                JOIN files ff ON ff.id = fn.file_id
                WHERE fn.file_id = ppc.file_id
                  AND fn.name = ?{next_property_param_index}
            "
        );
        property_params.push(Value::from(function_name.to_string()));
        next_property_param_index += 1;
        if let Some(class_name) = class_name {
            let _ = write!(
                property_sql,
                " AND fn.class_name = ?{next_property_param_index}"
            );
            property_params.push(Value::from(class_name.to_string()));
            next_property_param_index += 1;
        }
        if let Some(language) = self.ctx.language.as_ref() {
            let _ = write!(
                property_sql,
                " AND ff.language = ?{next_property_param_index}"
            );
            property_params.push(Value::from(language.to_string()));
        }
        property_sql.push_str(" )");
        property_sql.push_str(
            "
            ORDER BY f.path, ppc.start_line
            ",
        );
        (property_sql, property_params)
    }
}
