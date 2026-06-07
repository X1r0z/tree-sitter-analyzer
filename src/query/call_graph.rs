use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::params;

use super::call_edges::IndexedFunction;
use super::call_resolver::{
    AncestorsByFileClass, CallTargetResolver, FieldTypeCache, ParamTypeCache,
};
use super::{CallEdgeQuery, QueryContext};
use crate::models::{
    CallGraphPath, FunctionInfo, FunctionKey, GraphDirection, GraphPathNode, Location,
};
use crate::parser::call_targets::{
    has_non_self_object_target, matches_call_target, type_matches_class,
};
use crate::traversal::{collect_paths_dfs, TraversalPathStep};

#[derive(Clone, Eq, Hash, PartialEq)]
struct CallSite {
    file: String,
    line: usize,
}

#[derive(Clone)]
struct GraphNeighbor {
    node: IndexedFunction,
    call_site: CallSite,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct GraphEdgeKey {
    node: FunctionKey,
    call_site: CallSite,
}

struct GraphTraversalState {
    neighbors: HashMap<(GraphDirection, FunctionKey), Vec<GraphNeighbor>>,
    nodes: HashMap<(String, Option<String>), std::sync::Arc<[IndexedFunction]>>,
    field_types: FieldTypeCache,
    param_types: ParamTypeCache,
    ancestors: AncestorsByFileClass,
    properties: HashMap<(String, Option<String>), bool>,
}

type IndexedResolutionCache =
    HashMap<(i64, String, String, Option<String>, usize), Option<IndexedFunction>>;

struct GraphTraversalCaches<'a> {
    nodes: &'a mut HashMap<(String, Option<String>), std::sync::Arc<[IndexedFunction]>>,
    resolutions: &'a mut IndexedResolutionCache,
    field_types: &'a mut FieldTypeCache,
    param_types: &'a mut ParamTypeCache,
    ancestors: &'a mut AncestorsByFileClass,
    properties: &'a mut HashMap<(String, Option<String>), bool>,
}

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
        let start_nodes = edge_query.load_functions_by_name_class(function_name, class_name)?;
        if start_nodes.is_empty() {
            anyhow::bail!("Function '{function_name}' not found");
        }

        let mut state = GraphTraversalState {
            neighbors: HashMap::new(),
            nodes: HashMap::new(),
            field_types: HashMap::new(),
            param_types: HashMap::new(),
            ancestors: HashMap::new(),
            properties: HashMap::new(),
        };
        let mut resolutions = HashMap::new();

        let neighbors = &mut state.neighbors;
        let mut traversal_caches = GraphTraversalCaches {
            nodes: &mut state.nodes,
            resolutions: &mut resolutions,
            field_types: &mut state.field_types,
            param_types: &mut state.param_types,
            ancestors: &mut state.ancestors,
            properties: &mut state.properties,
        };

        let mut results = collect_paths_dfs(
            &start_nodes,
            direction,
            max_depth,
            IndexedFunction::key,
            |current| {
                let cache_key = (direction, current.key());
                let neighbors = if let Some(cached) = neighbors.get(&cache_key) {
                    cached.clone()
                } else {
                    let loaded =
                        self.load_graph_neighbors(current, direction, &mut traversal_caches)?;
                    neighbors.insert(cache_key, loaded.clone());
                    loaded
                };

                Ok::<Vec<(IndexedFunction, CallSite)>, anyhow::Error>(
                    neighbors
                        .into_iter()
                        .map(|neighbor| (neighbor.node, neighbor.call_site))
                        .collect(),
                )
            },
            |steps: &[TraversalPathStep<IndexedFunction, CallSite>]| {
                steps
                    .iter()
                    .map(|step| (step.node.key(), step.edge.clone()))
                    .collect::<Vec<_>>()
            },
            Self::build_graph,
        )?;

        results.sort_by(|left, right| {
            left.stacktrace
                .cmp(&right.stacktrace)
                .then(left.depth.cmp(&right.depth))
                .then(left.path.len().cmp(&right.path.len()))
        });
        Ok(results)
    }

    fn build_graph(
        direction: GraphDirection,
        steps: &[TraversalPathStep<IndexedFunction, CallSite>],
    ) -> CallGraphPath {
        let path: Vec<GraphPathNode> = steps
            .iter()
            .map(|step| GraphPathNode::from(&step.node.function))
            .collect();
        let stacktrace = direction.order_stacktrace(
            path.iter()
                .zip(steps.iter())
                .map(|(node, step)| {
                    let (file, line) = step.edge.as_ref().map_or(
                        (node.location.file.as_str(), node.location.start_line),
                        |call_site| (call_site.file.as_str(), call_site.line),
                    );
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
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        if node.function_id < 0 {
            return Ok(Vec::new());
        }
        match direction {
            GraphDirection::Forward => self.load_forward_neighbors(node, caches),
            GraphDirection::Backward => self.load_backward_neighbors(node, caches),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn load_forward_neighbors(
        &self,
        node: &IndexedFunction,
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let resolver = CallTargetResolver::new(self.ctx);
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

        let mut seen = HashSet::new();
        let mut results = Vec::new();
        for (callee_name, object_name, line) in rows {
            let enclosing = edge_query.resolve_enclosing_function(
                node.file_id,
                &node.function.location.file,
                &node.function.name,
                node.function.class_name.as_deref(),
                line,
                &mut super::call_edges::EnclosingFunctionCaches {
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
                let key = GraphEdgeKey {
                    node: candidate.key(),
                    call_site: CallSite {
                        file: node.function.location.file.clone(),
                        line,
                    },
                };
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
                let key = GraphEdgeKey {
                    node: candidate.key(),
                    call_site: CallSite {
                        file: node.function.location.file.clone(),
                        line,
                    },
                };
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

    #[allow(clippy::too_many_lines)]
    fn load_backward_neighbors(
        &self,
        node: &IndexedFunction,
        caches: &mut GraphTraversalCaches<'_>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let edge_query = CallEdgeQuery::new(self.ctx);
        let resolver = CallTargetResolver::new(self.ctx);
        let rows = self.load_backward_call_rows(&node.function.name)?;

        let mut seen = HashSet::new();
        let mut results = Vec::new();
        let unique_method_target = match node.function.class_name.as_deref() {
            Some(class_name) => {
                resolver.has_unique_method_target(&node.function.name, class_name)?
            }
            None => false,
        };
        for (file_id, file, caller_name, caller_class_name, object_name, line) in rows {
            if let Some(caller_name) = caller_name.as_deref() {
                let caller = edge_query.resolve_enclosing_function(
                    file_id,
                    &file,
                    caller_name,
                    caller_class_name.as_deref(),
                    line,
                    &mut super::call_edges::EnclosingFunctionCaches {
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
                    let key = GraphEdgeKey {
                        node: caller.key(),
                        call_site: CallSite {
                            file: file.clone(),
                            line,
                        },
                    };
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
            let key = GraphEdgeKey {
                node: caller.key(),
                call_site: CallSite {
                    file: file.clone(),
                    line,
                },
            };
            if seen.insert(key) {
                results.push(GraphNeighbor {
                    node: caller,
                    call_site: CallSite { file, line },
                });
            }
        }

        let property_key = (node.function.name.clone(), node.function.class_name.clone());
        let is_property = if let Some(value) = caches.properties.get(&property_key) {
            *value
        } else {
            let value = resolver
                .is_python_property(&node.function.name, node.function.class_name.as_deref())?;
            caches.properties.insert(property_key.clone(), value);
            value
        };

        if is_property {
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
                    let key = GraphEdgeKey {
                        node: caller.key(),
                        call_site: CallSite {
                            file: file.clone(),
                            line,
                        },
                    };
                    if seen.insert(key) {
                        results.push(GraphNeighbor {
                            node: caller,
                            call_site: CallSite { file, line },
                        });
                    }
                    continue;
                }

                if let Some(caller) = edge_query.resolve_enclosing_function(
                    file_id,
                    &file,
                    &caller_name,
                    caller_class_name.as_deref(),
                    line,
                    &mut super::call_edges::EnclosingFunctionCaches {
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
                    let key = GraphEdgeKey {
                        node: caller.key(),
                        call_site: CallSite {
                            file: file.clone(),
                            line,
                        },
                    };
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
