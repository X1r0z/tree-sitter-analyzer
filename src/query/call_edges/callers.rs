use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::params_from_iter;

use super::super::call_resolver::{AncestorsByFileClass, CallTargetResolver};
use super::{CallEdgeQuery, CallerRow, EnclosingFunctionCaches};
use crate::models::{CallerInfo, Location};
use crate::parser::call_targets::{
    has_non_self_object_target, matches_call_target, split_function_target, type_matches_class,
};

impl CallEdgeQuery<'_> {
    pub(crate) fn find_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CallerInfo>> {
        let mut callers = self.collect_callers(function_name, class_name)?;
        callers.append(&mut self.collect_property_callers(function_name, class_name)?);
        let mut seen = HashSet::new();
        callers.retain(|caller| {
            seen.insert((
                caller.location.file.clone(),
                caller.caller.clone(),
                caller.location.start_line,
                caller.location.end_line,
            ))
        });
        callers.sort_by(|left, right| {
            left.location
                .file
                .cmp(&right.location.file)
                .then(left.location.start_line.cmp(&right.location.start_line))
                .then(left.caller.cmp(&right.caller))
        });
        Ok(callers)
    }

    fn collect_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CallerInfo>> {
        let resolver = CallTargetResolver::new(self.ctx);
        let (target_function, target_object) = split_function_target(function_name);
        let (sql, params) = self.build_caller_sql(target_function);
        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
            Ok(CallerRow {
                file_id: row.get(0)?,
                file: row.get(1)?,
                caller: row.get(2)?,
                caller_class_name: row.get(3)?,
                object_name: row.get(4)?,
                start_line: row.get::<_, usize>(5)?,
                end_line: row.get::<_, usize>(6)?,
            })
        })?;

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
        let mut callers = Vec::new();
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
                } else if !(matches_call_target(
                    row.caller_class_name.as_deref(),
                    row.object_name.as_deref(),
                    class_name,
                    |_attr_name, _target_class_name| false,
                    |_attr_name, _target_class_name| false,
                ) || unique_method_target
                    && has_non_self_object_target(row.object_name.as_deref()))
                {
                    continue;
                }
            }
            let caller = row.caller.unwrap_or_else(|| "<module>".to_string());
            callers.push(CallerInfo {
                caller,
                location: Location {
                    file: row.file,
                    start_line: row.start_line,
                    end_line: row.end_line,
                },
            });
        }

        Ok(callers)
    }

    fn collect_property_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CallerInfo>> {
        let resolver = CallTargetResolver::new(self.ctx);
        if !resolver.is_python_property(function_name, class_name)? {
            return Ok(Vec::new());
        }
        let (target_function, _target_object) = split_function_target(function_name);
        let (property_sql, property_params) = self.build_property_caller_sql(target_function);

        let mut field_type_cache = HashMap::new();
        let mut param_type_cache = HashMap::new();
        let mut ancestors_cache: AncestorsByFileClass = HashMap::new();
        let mut node_cache = HashMap::new();
        let mut resolution_cache = HashMap::new();
        let mut enclosing_caches = EnclosingFunctionCaches {
            candidates: &mut node_cache,
            resolutions: &mut resolution_cache,
        };

        let mut property_stmt = self.ctx.conn.prepare(&property_sql)?;
        let property_rows =
            property_stmt.query_map(params_from_iter(property_params.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, usize>(6)?,
                    row.get::<_, usize>(7)?,
                ))
            })?;
        let mut callers = Vec::new();
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
                    if object_name.as_deref() == Some(class_name)
                        || !type_matches_class(object_type.as_deref(), class_name)
                    {
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
                        &mut ancestors_cache,
                    )? {
                        continue;
                    }
                }
            }
            callers.push(CallerInfo {
                caller: caller_name,
                location: Location {
                    file,
                    start_line,
                    end_line,
                },
            });
        }

        Ok(callers)
    }

    fn build_caller_sql(&self, target_function: &str) -> (String, Vec<Value>) {
        let mut sql = String::from(
            "
            SELECT c.file_id, f.path, c.caller, c.caller_class_name, c.object_name, c.start_line, c.end_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.callee = ?1
            ",
        );
        let mut params = vec![Value::from(target_function.to_string())];
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(Value::from(language.to_string()));
        }
        sql.push_str(" ORDER BY f.path, c.start_line");
        (sql, params)
    }

    fn build_property_caller_sql(&self, target_function: &str) -> (String, Vec<Value>) {
        let mut property_sql = String::from(
            "
            SELECT ppc.file_id, f.path, ppc.caller, ppc.caller_class_name, ppc.object_name, ppc.object_type, ppc.start_line, ppc.end_line
            FROM python_property_callers ppc
            JOIN files f ON f.id = ppc.file_id
            WHERE ppc.property_name = ?1
            ",
        );
        let mut property_params = vec![Value::from(target_function.to_string())];
        if let Some(language) = self.ctx.language.as_ref() {
            property_sql.push_str(" AND f.language = ?2");
            property_params.push(Value::from(language.to_string()));
        }
        property_sql.push_str(" ORDER BY f.path, ppc.start_line");
        (property_sql, property_params)
    }
}
