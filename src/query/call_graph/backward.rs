use std::collections::HashSet;

use rusqlite::params;

use super::super::call_edges::{EnclosingFunctionCaches, IndexedFunction};
use super::super::call_resolver::CallTargetResolver;
use super::super::CallEdgeQuery;
use super::{CallGraphQuery, CallSite, GraphNeighbor, GraphTraversalCaches};
use crate::models::{FunctionInfo, Location};
use crate::parser::call_targets::{
    has_non_self_object_target, matches_call_target, type_matches_class,
};

type BackwardCallRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    usize,
);

type BackwardPropertyRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    usize,
);

impl CallGraphQuery<'_> {
    pub(super) fn load_backward_neighbors(
        &self,
        node: &IndexedFunction,
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let mut neighbors = self.collect_backward_calls(node, caches)?;
        neighbors.append(&mut self.collect_backward_properties(node, caches)?);
        let mut seen = HashSet::new();
        neighbors.retain(|neighbor| seen.insert(neighbor.edge_key()));
        Ok(neighbors)
    }

    fn collect_backward_calls(
        &self,
        node: &IndexedFunction,
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let resolver = CallTargetResolver::new(self.ctx);
        let rows = self.load_backward_call_rows(&node.function.name)?;
        let unique_method_target = match node.function.class_name.as_deref() {
            Some(class_name) => {
                resolver.has_unique_method_target(&node.function.name, class_name)?
            }
            None => false,
        };
        let mut neighbors = Vec::new();
        for (file_id, file, caller_name, caller_class_name, object_name, line) in rows {
            if let Some(caller_name) = caller_name.as_deref() {
                let caller = edge_query.resolve_enclosing_function(
                    file_id,
                    &file,
                    caller_name,
                    caller_class_name.as_deref(),
                    line,
                    &mut EnclosingFunctionCaches {
                        candidates: caches.nodes,
                        resolutions: caches.resolutions,
                    },
                )?;
                if let Some(caller) = caller {
                    if let Some(class_name) = node.function.class_name.as_deref() {
                        if !(resolver.matches_call_target(
                            &caller,
                            object_name.as_deref(),
                            class_name,
                            caches.field_types,
                            caches.param_types,
                        )? || unique_method_target
                            && has_non_self_object_target(object_name.as_deref()))
                        {
                            continue;
                        }
                    }
                    neighbors.push(GraphNeighbor {
                        node: caller,
                        call_site: CallSite { file, line },
                    });
                }
                continue;
            }

            if let Some(class_name) = node.function.class_name.as_deref() {
                if !(matches_call_target(
                    caller_class_name.as_deref(),
                    object_name.as_deref(),
                    class_name,
                    |_attr_name, _target_class_name| false,
                    |_attr_name, _target_class_name| false,
                ) || unique_method_target
                    && has_non_self_object_target(object_name.as_deref()))
                {
                    continue;
                }
            }

            let caller = IndexedFunction {
                function_id: -1,
                file_id,
                function: FunctionInfo {
                    name: "<module>".to_string(),
                    location: Location {
                        file: file.clone(),
                        start_line: line,
                        end_line: line,
                    },
                    body: String::new(),
                    class_name: None,
                    params: Vec::new(),
                },
            };
            neighbors.push(GraphNeighbor {
                node: caller,
                call_site: CallSite { file, line },
            });
        }

        Ok(neighbors)
    }

    fn collect_backward_properties(
        &self,
        node: &IndexedFunction,
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let resolver = CallTargetResolver::new(self.ctx);
        let property_key = (node.function.name.clone(), node.function.class_name.clone());
        let is_property = if let Some(value) = caches.properties.get(&property_key) {
            *value
        } else {
            let value = resolver
                .is_python_property(&node.function.name, node.function.class_name.as_deref())?;
            caches.properties.insert(property_key.clone(), value);
            value
        };
        if !is_property {
            return Ok(Vec::new());
        }

        let mut neighbors = Vec::new();
        for (file_id, file, caller_name, caller_class_name, object_name, object_type, line) in
            self.load_backward_property_rows(&node.function.name)?
        {
            if caller_name == "<module>" {
                if let Some(class_name) = node.function.class_name.as_deref() {
                    if object_name.as_deref() == Some(class_name)
                        || !type_matches_class(object_type.as_deref(), class_name)
                    {
                        continue;
                    }
                }
                let caller = IndexedFunction {
                    function_id: -1,
                    file_id,
                    function: FunctionInfo {
                        name: "<module>".to_string(),
                        location: Location {
                            file: file.clone(),
                            start_line: line,
                            end_line: line,
                        },
                        body: String::new(),
                        class_name: None,
                        params: Vec::new(),
                    },
                };
                neighbors.push(GraphNeighbor {
                    node: caller,
                    call_site: CallSite { file, line },
                });
                continue;
            }

            if let Some(caller) = edge_query.resolve_enclosing_function(
                file_id,
                &file,
                &caller_name,
                caller_class_name.as_deref(),
                line,
                &mut EnclosingFunctionCaches {
                    candidates: caches.nodes,
                    resolutions: caches.resolutions,
                },
            )? {
                if !resolver.matches_property_target(
                    Some(&caller),
                    object_name.as_deref(),
                    node.function.class_name.as_deref().unwrap_or_default(),
                    caches.field_types,
                    caches.param_types,
                    caches.ancestors,
                )? {
                    continue;
                }
                neighbors.push(GraphNeighbor {
                    node: caller,
                    call_site: CallSite { file, line },
                });
            }
        }

        Ok(neighbors)
    }

    fn load_backward_call_rows(&self, function_name: &str) -> anyhow::Result<Vec<BackwardCallRow>> {
        let rows = if let Some(language) = self.ctx.language.as_ref() {
            let mut stmt = self.ctx.conn.prepare_cached(
                "
                SELECT c.file_id, f.path, c.caller, c.caller_class_name, c.object_name, c.start_line
                FROM calls c
                JOIN files f ON f.id = c.file_id
                WHERE c.callee = ?1 AND f.language = ?2
                ORDER BY f.path, c.start_line
                ",
            )?;
            let rows = stmt.query_map(params![function_name, language], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, usize>(5)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.ctx.conn.prepare_cached(
                "
                SELECT c.file_id, f.path, c.caller, c.caller_class_name, c.object_name, c.start_line
                FROM calls c
                JOIN files f ON f.id = c.file_id
                WHERE c.callee = ?1
                ORDER BY f.path, c.start_line
                ",
            )?;
            let rows = stmt.query_map(params![function_name], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, usize>(5)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        Ok(rows)
    }

    fn load_backward_property_rows(
        &self,
        property_name: &str,
    ) -> anyhow::Result<Vec<BackwardPropertyRow>> {
        let rows = if let Some(language) = self.ctx.language.as_ref() {
            let mut stmt = self.ctx.conn.prepare_cached(
                "
                SELECT ppc.file_id, f.path, ppc.caller, ppc.caller_class_name, ppc.object_name, ppc.object_type, ppc.start_line
                FROM python_property_callers ppc
                JOIN files f ON f.id = ppc.file_id
                WHERE ppc.property_name = ?1 AND f.language = ?2
                ORDER BY f.path, ppc.start_line
                ",
            )?;
            let rows = stmt.query_map(params![property_name, language], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, usize>(6)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.ctx.conn.prepare_cached(
                "
                SELECT ppc.file_id, f.path, ppc.caller, ppc.caller_class_name, ppc.object_name, ppc.object_type, ppc.start_line
                FROM python_property_callers ppc
                JOIN files f ON f.id = ppc.file_id
                WHERE ppc.property_name = ?1
                ORDER BY f.path, ppc.start_line
                ",
            )?;
            let rows = stmt.query_map(params![property_name], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, usize>(6)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        Ok(rows)
    }
}
