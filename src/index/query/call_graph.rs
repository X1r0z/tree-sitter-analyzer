use std::collections::{HashMap, HashSet};

use rusqlite::{params, params_from_iter, ToSql};

use super::call_edges::IndexedFunction;
use super::call_resolver::{CallTargetResolver, FieldTypeCache, ParamTypeCache};
use super::{CallEdgeQuery, QueryContext};
use crate::models::{CallGraphPath, FunctionKey, GraphDirection, GraphPathNode};
use crate::traversal::dfs::{try_collect_paths, PathStep};

#[derive(Clone)]
struct CallSite {
    file: String,
    line: usize,
}

#[derive(Clone)]
struct GraphNeighbor {
    node: IndexedFunction,
    call_site: CallSite,
}

struct GraphTraversalState {
    neighbor_cache: HashMap<(GraphDirection, FunctionKey), Vec<GraphNeighbor>>,
    node_cache: HashMap<(String, Option<String>), Vec<IndexedFunction>>,
    field_type_cache: FieldTypeCache,
    param_type_cache: ParamTypeCache,
    is_property_cache: HashMap<(String, Option<String>), bool>,
}

pub(crate) struct CallGraphQuery<'a> {
    ctx: QueryContext<'a>,
}

impl<'a> CallGraphQuery<'a> {
    pub(crate) fn new(ctx: QueryContext<'a>) -> Self {
        Self { ctx }
    }

    pub(crate) fn find_graphs(
        &self,
        function_name: &str,
        class_name: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
    ) -> anyhow::Result<Vec<CallGraphPath>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let start_nodes = edge_query.load_exact_functions(function_name, class_name)?;
        if start_nodes.is_empty() {
            anyhow::bail!("Function '{}' not found", function_name);
        }

        let mut state = GraphTraversalState {
            neighbor_cache: HashMap::new(),
            node_cache: HashMap::new(),
            field_type_cache: HashMap::new(),
            param_type_cache: HashMap::new(),
            is_property_cache: HashMap::new(),
        };

        let neighbor_cache = &mut state.neighbor_cache;
        let node_cache = &mut state.node_cache;
        let field_type_cache = &mut state.field_type_cache;
        let param_type_cache = &mut state.param_type_cache;
        let is_property_cache = &mut state.is_property_cache;

        let mut results = try_collect_paths(
            &start_nodes,
            direction,
            max_depth,
            IndexedFunction::key,
            |current| {
                let cache_key = (direction, current.key());
                let neighbors = if let Some(cached) = neighbor_cache.get(&cache_key) {
                    cached.clone()
                } else {
                    let loaded = self.load_graph_neighbors(
                        current,
                        direction,
                        node_cache,
                        field_type_cache,
                        param_type_cache,
                        is_property_cache,
                    )?;
                    neighbor_cache.insert(cache_key, loaded.clone());
                    loaded
                };

                Ok::<Vec<(IndexedFunction, CallSite)>, anyhow::Error>(
                    neighbors
                        .into_iter()
                        .map(|neighbor| (neighbor.node, neighbor.call_site))
                        .collect(),
                )
            },
            Self::materialize_graph,
        )?;

        results.sort_by(|left, right| {
            left.stacktrace
                .cmp(&right.stacktrace)
                .then(left.depth.cmp(&right.depth))
                .then(left.path.len().cmp(&right.path.len()))
        });
        Ok(results)
    }

    fn materialize_graph(
        direction: GraphDirection,
        steps: &[PathStep<IndexedFunction, CallSite>],
    ) -> CallGraphPath {
        let path: Vec<GraphPathNode> = steps
            .iter()
            .map(|step| GraphPathNode::from(&step.node.function))
            .collect();
        let stacktrace = direction.order_stacktrace(
            path.iter()
                .zip(steps.iter())
                .map(|(node, step)| {
                    let (file, line) = step
                        .edge
                        .as_ref()
                        .map(|call_site| (call_site.file.as_str(), call_site.line))
                        .unwrap_or((node.file.as_str(), node.start_line));
                    node.stacktrace_name(file, line)
                })
                .collect(),
        );
        CallGraphPath {
            depth: path.len().saturating_sub(1),
            stacktrace,
            path,
        }
    }

    fn load_graph_neighbors(
        &self,
        node: &IndexedFunction,
        direction: GraphDirection,
        node_cache: &mut HashMap<(String, Option<String>), Vec<IndexedFunction>>,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
        is_property_cache: &mut HashMap<(String, Option<String>), bool>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        match direction {
            GraphDirection::Forward => {
                self.load_forward_neighbors(node, node_cache, field_type_cache, param_type_cache)
            }
            GraphDirection::Backward => self.load_backward_neighbors(
                node,
                node_cache,
                field_type_cache,
                param_type_cache,
                is_property_cache,
            ),
        }
    }

    fn load_forward_neighbors(
        &self,
        node: &IndexedFunction,
        node_cache: &mut HashMap<(String, Option<String>), Vec<IndexedFunction>>,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let resolver = CallTargetResolver::new(self.ctx);
        let start_line = node.function.location.start_line as i64;
        let end_line = node.function.location.end_line as i64;
        let mut sql = String::from(
            "
            SELECT callee, object_name, start_line
            FROM calls
            WHERE file_id = ?1 AND caller = ?2 AND start_line >= ?3 AND start_line <= ?4
            ",
        );

        let rows = match node.function.class_name.as_deref() {
            Some(class_name) => {
                sql.push_str(" AND caller_class_name = ?5");
                sql.push_str(" ORDER BY start_line");
                let mut stmt = self.ctx.conn.prepare(&sql)?;
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
                            row.get::<_, i64>(2)? as usize,
                        ))
                    },
                )?;
                rows.collect::<Result<Vec<_>, _>>()?
            }
            None => {
                sql.push_str(" AND caller_class_name IS NULL");
                sql.push_str(" ORDER BY start_line");
                let mut stmt = self.ctx.conn.prepare(&sql)?;
                let rows = stmt.query_map(
                    params![node.file_id, node.function.name, start_line, end_line],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, i64>(2)? as usize,
                        ))
                    },
                )?;
                rows.collect::<Result<Vec<_>, _>>()?
            }
        };

        let mut seen = HashSet::new();
        let mut results = Vec::new();
        for (callee_name, object_name, line) in rows {
            let candidates = edge_query.load_functions_by_name(&callee_name, node_cache)?;
            for candidate in resolver.resolve_forward_targets(
                node,
                object_name.as_deref(),
                &candidates,
                field_type_cache,
                param_type_cache,
            )? {
                let key = candidate.key();
                if seen.insert(key) {
                    results.push(GraphNeighbor {
                        node: candidate,
                        call_site: CallSite {
                            file: node.function.location.file.clone(),
                            line,
                        },
                    });
                }
            }
        }
        Ok(results)
    }

    fn load_backward_neighbors(
        &self,
        node: &IndexedFunction,
        node_cache: &mut HashMap<(String, Option<String>), Vec<IndexedFunction>>,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
        is_property_cache: &mut HashMap<(String, Option<String>), bool>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let resolver = CallTargetResolver::new(self.ctx);
        let mut sql = String::from(
            "
            SELECT c.file_id, f.path, c.caller, c.caller_class_name, c.object_name, c.start_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.callee = ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&node.function.name];
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, c.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)? as usize,
            ))
        })?;

        let mut seen = HashSet::new();
        let mut results = Vec::new();
        for row in rows {
            let (file_id, file, caller_name, caller_class_name, object_name, line) = row?;
            let Some(caller_name) = caller_name.as_deref() else {
                continue;
            };
            let caller = edge_query.resolve_enclosing_function(
                file_id,
                &file,
                caller_name,
                caller_class_name.as_deref(),
                line,
                node_cache,
            )?;
            if let Some(caller) = caller {
                if let Some(class_name) = node.function.class_name.as_deref() {
                    if !resolver.matches_call_target(
                        &caller,
                        object_name.as_deref(),
                        class_name,
                        field_type_cache,
                        param_type_cache,
                    )? {
                        continue;
                    }
                }
                let key = caller.key();
                if seen.insert(key) {
                    results.push(GraphNeighbor {
                        node: caller,
                        call_site: CallSite {
                            file: file.clone(),
                            line,
                        },
                    });
                }
            }
        }

        let property_key = (node.function.name.clone(), node.function.class_name.clone());
        let is_property = if let Some(value) = is_property_cache.get(&property_key) {
            *value
        } else {
            let value = resolver
                .is_python_property(&node.function.name, node.function.class_name.as_deref())?;
            is_property_cache.insert(property_key.clone(), value);
            value
        };

        if is_property {
            let mut property_sql = String::from(
                "
                SELECT ppc.file_id, f.path, ppc.caller, ppc.caller_class_name, ppc.object_name, ppc.line
                FROM python_property_callers ppc
                JOIN files f ON f.id = ppc.file_id
                WHERE ppc.property_name = ?1
                ",
            );
            let mut property_params: Vec<&dyn ToSql> = vec![&node.function.name];
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
                if let Some(caller) = edge_query.resolve_enclosing_function(
                    file_id,
                    &file,
                    &caller_name,
                    caller_class_name.as_deref(),
                    line,
                    node_cache,
                )? {
                    if !resolver.matches_property_target(
                        Some(&caller),
                        object_name.as_deref(),
                        node.function.class_name.as_deref().unwrap_or_default(),
                        field_type_cache,
                        param_type_cache,
                    )? {
                        continue;
                    }
                    let key = caller.key();
                    if seen.insert(key) {
                        results.push(GraphNeighbor {
                            node: caller,
                            call_site: CallSite { file, line },
                        });
                    }
                }
            }
        }

        Ok(results)
    }
}
