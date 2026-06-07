use std::collections::HashMap;

use super::call_edges::IndexedFunction;
use super::call_resolver::{AncestorsByFileClass, FieldTypeCache, ParamTypeCache};
use super::{CallEdgeQuery, QueryContext};
use crate::models::{CallGraphPath, FunctionKey, GraphDirection, GraphPathNode};
use crate::traversal::{collect_paths_dfs, TraversalPathStep};

mod backward;
mod forward;

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
}
