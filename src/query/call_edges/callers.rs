use std::collections::{HashMap, HashSet};

use rusqlite::{params_from_iter, ToSql};

use super::super::call_resolver::{AncestorsByFileClass, CallTargetResolver};
use super::{CallEdgeQuery, CallerRow, EnclosingFunctionCaches};
use crate::models::{CallerInfo, Location};
use crate::parser::call_targets::{
    has_non_self_object_target, matches_call_target, split_function_target, type_matches_class,
};
use crate::utils::sort_callers_by_file_line;

impl CallEdgeQuery<'_> {
    #[allow(clippy::too_many_lines)]
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
                start_line: row.get::<_, usize>(5)?,
                end_line: row.get::<_, usize>(6)?,
            })
        })?;

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        let mut field_type_cache = HashMap::new();
        let mut param_type_cache = HashMap::new();
        let mut ancestors_cache: AncestorsByFileClass = HashMap::new();
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
                        row.get::<_, usize>(6)?,
                        row.get::<_, usize>(7)?,
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
}
