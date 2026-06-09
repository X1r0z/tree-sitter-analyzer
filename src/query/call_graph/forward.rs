use std::collections::{BTreeSet, HashSet};

use rusqlite::params;

use super::super::call_edges::{EnclosingFunctionCaches, IndexedFunction};
use super::super::call_resolver::CallTargetResolver;
use super::super::CallEdgeQuery;
use super::{CallGraphQuery, CallSite, GraphNeighbor, GraphTraversalCaches};
use crate::models::{FunctionInfo, Location};

impl CallGraphQuery<'_> {
    pub(super) fn load_forward_neighbors(
        &self,
        node: &IndexedFunction,
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let rows = self.load_forward_call_rows(node)?;
        let property_rows = self.load_forward_property_rows(node)?;
        let mut callee_names = BTreeSet::new();
        callee_names.extend(rows.iter().map(|(callee_name, _, _)| callee_name.clone()));
        callee_names.extend(
            property_rows
                .iter()
                .map(|(property_name, _, _)| property_name.clone()),
        );
        edge_query.load_functions_by_names(&callee_names, caches.nodes)?;

        let mut neighbors = self.collect_forward_calls(node, rows, caches)?;
        neighbors.append(&mut self.collect_forward_properties(node, property_rows, caches)?);
        let mut seen = HashSet::new();
        neighbors.retain(|neighbor| seen.insert(neighbor.edge_key()));
        Ok(neighbors)
    }

    fn collect_forward_calls(
        &self,
        node: &IndexedFunction,
        rows: Vec<(String, Option<String>, usize)>,
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let resolver = CallTargetResolver::new(self.ctx);
        let mut neighbors = Vec::new();
        for (callee_name, object_name, line) in rows {
            let enclosing = edge_query.resolve_enclosing_function(
                node.file_id,
                &node.function.location.file,
                &node.function.name,
                node.function.class_name.as_deref(),
                line,
                &mut EnclosingFunctionCaches {
                    candidates: caches.nodes,
                    resolutions: caches.resolutions,
                },
            )?;
            if !enclosing.is_some_and(|caller| caller.key() == node.key()) {
                continue;
            }
            let candidates = edge_query.load_functions_by_name(&callee_name, caches.nodes)?;
            let mut resolved_targets = resolver.resolve_forward_targets(
                node,
                object_name.as_deref(),
                candidates.as_ref(),
                caches.field_types,
                caches.param_types,
            )?;
            if resolved_targets.is_empty() {
                let name = match object_name.as_deref() {
                    Some(object_name) => format!("{object_name}.{callee_name}"),
                    None => callee_name.clone(),
                };
                resolved_targets.push(IndexedFunction {
                    function_id: -1,
                    file_id: node.file_id,
                    function: FunctionInfo {
                        name,
                        location: Location {
                            file: node.function.location.file.clone(),
                            start_line: line,
                            end_line: line,
                        },
                        body: String::new(),
                        class_name: None,
                        params: Vec::new(),
                    },
                });
            }
            for candidate in resolved_targets {
                neighbors.push(GraphNeighbor {
                    node: candidate,
                    call_site: CallSite {
                        file: node.function.location.file.clone(),
                        line,
                    },
                });
            }
        }

        Ok(neighbors)
    }

    fn collect_forward_properties(
        &self,
        node: &IndexedFunction,
        property_rows: Vec<(String, Option<String>, usize)>,
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let resolver = CallTargetResolver::new(self.ctx);
        let mut neighbors = Vec::new();
        for (property_name, object_name, line) in property_rows {
            let candidates = edge_query.load_functions_by_name(&property_name, caches.nodes)?;
            let mut matched = Vec::new();
            for candidate in candidates.iter() {
                let property_key = (
                    candidate.function.name.clone(),
                    candidate.function.class_name.clone(),
                );
                let is_property = if let Some(value) = caches.properties.get(&property_key) {
                    *value
                } else {
                    let value = resolver.is_python_property(
                        &candidate.function.name,
                        candidate.function.class_name.as_deref(),
                    )?;
                    caches.properties.insert(property_key.clone(), value);
                    value
                };
                if !is_property {
                    continue;
                }
                if !resolver.matches_property_target(
                    Some(node),
                    object_name.as_deref(),
                    candidate.function.class_name.as_deref().unwrap_or_default(),
                    caches.field_types,
                    caches.param_types,
                    caches.ancestors,
                )? {
                    continue;
                }
                matched.push(candidate.clone());
            }

            if matched.is_empty() {
                let name = match object_name.as_deref() {
                    Some(object_name) => format!("{object_name}.{property_name}"),
                    None => property_name.clone(),
                };
                matched.push(IndexedFunction {
                    function_id: -1,
                    file_id: node.file_id,
                    function: FunctionInfo {
                        name,
                        location: Location {
                            file: node.function.location.file.clone(),
                            start_line: line,
                            end_line: line,
                        },
                        body: String::new(),
                        class_name: None,
                        params: Vec::new(),
                    },
                });
            }

            for candidate in matched {
                neighbors.push(GraphNeighbor {
                    node: candidate,
                    call_site: CallSite {
                        file: node.function.location.file.clone(),
                        line,
                    },
                });
            }
        }

        Ok(neighbors)
    }

    fn load_forward_call_rows(
        &self,
        node: &IndexedFunction,
    ) -> anyhow::Result<Vec<(String, Option<String>, usize)>> {
        let start_line = node.function.location.start_line;
        let end_line = node.function.location.end_line;
        let rows = if let Some(class_name) = node.function.class_name.as_deref() {
            let mut stmt = self.ctx.conn.prepare_cached(
                "
                SELECT callee, object_name, start_line
                FROM calls
                WHERE file_id = ?1 AND caller = ?2 AND start_line >= ?3 AND start_line <= ?4
                  AND caller_class_name = ?5
                ORDER BY start_line
                ",
            )?;
            let rows = stmt.query_map(
                params![
                    node.file_id,
                    node.function.name,
                    start_line,
                    end_line,
                    class_name
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, usize>(2)?,
                    ))
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.ctx.conn.prepare_cached(
                "
                SELECT callee, object_name, start_line
                FROM calls
                WHERE file_id = ?1 AND caller = ?2 AND start_line >= ?3 AND start_line <= ?4
                  AND caller_class_name IS NULL
                ORDER BY start_line
                ",
            )?;
            let rows = stmt.query_map(
                params![node.file_id, node.function.name, start_line, end_line],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, usize>(2)?,
                    ))
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        Ok(rows)
    }

    fn load_forward_property_rows(
        &self,
        node: &IndexedFunction,
    ) -> anyhow::Result<Vec<(String, Option<String>, usize)>> {
        let start_line = node.function.location.start_line;
        let end_line = node.function.location.end_line;
        let rows = if let Some(class_name) = node.function.class_name.as_deref() {
            let mut stmt = self.ctx.conn.prepare_cached(
                "
                SELECT property_name, object_name, start_line
                FROM python_property_callers
                WHERE file_id = ?1 AND caller = ?2 AND start_line >= ?3 AND start_line <= ?4
                  AND caller_class_name = ?5
                ORDER BY start_line
                ",
            )?;
            let rows = stmt.query_map(
                params![
                    node.file_id,
                    node.function.name,
                    start_line,
                    end_line,
                    class_name
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, usize>(2)?,
                    ))
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.ctx.conn.prepare_cached(
                "
                SELECT property_name, object_name, start_line
                FROM python_property_callers
                WHERE file_id = ?1 AND caller = ?2 AND start_line >= ?3 AND start_line <= ?4
                  AND caller_class_name IS NULL
                ORDER BY start_line
                ",
            )?;
            let rows = stmt.query_map(
                params![node.file_id, node.function.name, start_line, end_line],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, usize>(2)?,
                    ))
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        Ok(rows)
    }
}
