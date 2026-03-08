use std::collections::{HashMap, HashSet};

use crate::models::{
    CallGraphPath, CallInfo, FieldInfo, FunctionInfo, FunctionKey, GraphDirection, GraphPathNode,
    PythonPropertyInfo,
};
use crate::utils::{extract_instance_attr, type_matches_class};

#[derive(Debug, Clone)]
pub(crate) struct RawPropertyCaller {
    pub(crate) file: String,
    pub(crate) property_name: String,
    pub(crate) caller: String,
    pub(crate) line: usize,
}

#[derive(Debug, Clone)]
struct CallSite {
    file: String,
    line: usize,
}

#[derive(Debug, Clone)]
struct GraphNeighbor {
    key: FunctionKey,
    call_site: CallSite,
}

#[derive(Debug, Clone)]
struct GraphTraceStep {
    key: FunctionKey,
    call_site: Option<CallSite>,
}

#[derive(Debug, Clone)]
pub(crate) struct CallGraph {
    functions_by_key: HashMap<FunctionKey, FunctionInfo>,
    starting_keys: Vec<FunctionKey>,
    forward_edges: HashMap<FunctionKey, Vec<GraphNeighbor>>,
    backward_edges: HashMap<FunctionKey, Vec<GraphNeighbor>>,
}

impl CallGraph {
    pub(crate) fn build(
        functions: Vec<FunctionInfo>,
        fields: Vec<FieldInfo>,
        calls: Vec<CallInfo>,
        python_properties: Vec<PythonPropertyInfo>,
        property_callers: Vec<RawPropertyCaller>,
    ) -> Self {
        let mut functions_by_key = HashMap::new();
        let mut starting_keys = Vec::new();
        let mut defs_by_name: HashMap<String, Vec<FunctionInfo>> = HashMap::new();
        for function in functions {
            let key = FunctionKey::from_function(&function);
            starting_keys.push(key.clone());
            defs_by_name
                .entry(function.name.clone())
                .or_default()
                .push(function.clone());
            functions_by_key.insert(key, function);
        }

        let mut fields_by_file_class: HashMap<(String, String), Vec<FieldInfo>> = HashMap::new();
        for field in fields {
            if let Some(class_name) = field.class_name.clone() {
                fields_by_file_class
                    .entry((field.location.file.clone(), class_name))
                    .or_default()
                    .push(field);
            }
        }

        let property_keys: HashSet<(String, Option<String>)> = python_properties
            .into_iter()
            .map(|property| (property.name, property.class_name))
            .collect();

        let mut functions_by_file_context: HashMap<
            (String, String, Option<String>),
            Vec<FunctionInfo>,
        > = HashMap::new();
        for function in functions_by_key.values() {
            functions_by_file_context
                .entry((
                    function.location.file.clone(),
                    function.name.clone(),
                    function.class_name.clone(),
                ))
                .or_default()
                .push(function.clone());
        }
        for functions in functions_by_file_context.values_mut() {
            functions
                .sort_by_key(|function| (function.location.start_line, function.location.end_line));
        }

        let mut forward_edge_sets: HashMap<FunctionKey, HashMap<FunctionKey, CallSite>> =
            HashMap::new();
        for call in calls {
            let Some(caller) = resolve_enclosing_function(
                &functions_by_file_context,
                &call.location.file,
                call.caller.as_deref(),
                call.caller_class_name.as_deref(),
                call.location.start_line,
            ) else {
                continue;
            };

            for callee in resolve_call_targets(&call, &caller, &defs_by_name, &fields_by_file_class)
            {
                let caller_key = FunctionKey::from_function(&caller);
                let call_site = CallSite {
                    file: call.location.file.clone(),
                    line: call.location.start_line,
                };
                forward_edge_sets
                    .entry(caller_key)
                    .or_default()
                    .entry(callee)
                    .or_insert(call_site);
            }
        }

        for property_caller in property_callers {
            let Some(caller) = resolve_enclosing_function(
                &functions_by_file_context,
                &property_caller.file,
                Some(&property_caller.caller),
                None,
                property_caller.line,
            ) else {
                continue;
            };

            let candidate_defs = defs_by_name
                .get(&property_caller.property_name)
                .cloned()
                .unwrap_or_default();
            for property in candidate_defs {
                if !property_keys.contains(&(property.name.clone(), property.class_name.clone())) {
                    continue;
                }
                let caller_key = FunctionKey::from_function(&caller);
                let property_key = FunctionKey::from_function(&property);
                let call_site = CallSite {
                    file: property_caller.file.clone(),
                    line: property_caller.line,
                };
                forward_edge_sets
                    .entry(caller_key)
                    .or_default()
                    .entry(property_key)
                    .or_insert(call_site);
            }
        }

        let forward_edges = freeze_edges(forward_edge_sets);
        let mut backward_edge_sets: HashMap<FunctionKey, HashMap<FunctionKey, CallSite>> =
            HashMap::new();
        for (caller, callees) in &forward_edges {
            for callee in callees {
                backward_edge_sets
                    .entry(callee.key.clone())
                    .or_default()
                    .entry(caller.clone())
                    .or_insert_with(|| callee.call_site.clone());
            }
        }

        Self {
            functions_by_key,
            starting_keys,
            forward_edges,
            backward_edges: freeze_edges(backward_edge_sets),
        }
    }

    pub(crate) fn find_start_nodes(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionKey> {
        let mut matches: Vec<_> = self
            .starting_keys
            .iter()
            .filter_map(|key| {
                self.functions_by_key.get(key).and_then(|function| {
                    (function.name == function_name
                        && (class_name.is_none() || function.class_name.as_deref() == class_name))
                        .then(|| key.clone())
                })
            })
            .collect();
        matches.sort_by(compare_keys);
        matches
    }

    pub(crate) fn collect_graphs(
        &self,
        start_nodes: &[FunctionKey],
        direction: GraphDirection,
        max_depth: usize,
    ) -> Vec<CallGraphPath> {
        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        for start in start_nodes {
            let mut current_path = vec![GraphTraceStep {
                key: start.clone(),
                call_site: None,
            }];
            let mut visited = HashSet::from([start.clone()]);
            self.walk(
                direction,
                max_depth,
                &mut current_path,
                &mut visited,
                &mut paths,
                &mut seen,
            );
        }
        paths.sort_by(compare_graphs);
        paths
    }

    fn walk(
        &self,
        direction: GraphDirection,
        remaining_depth: usize,
        current_path: &mut Vec<GraphTraceStep>,
        visited: &mut HashSet<FunctionKey>,
        results: &mut Vec<CallGraphPath>,
        seen_paths: &mut HashSet<Vec<FunctionKey>>,
    ) {
        let current = current_path
            .last()
            .cloned()
            .unwrap_or_else(|| unreachable!());
        let neighbors = match direction {
            GraphDirection::Backward => self.backward_edges.get(&current.key),
            GraphDirection::Forward => self.forward_edges.get(&current.key),
        };

        let next_nodes: Vec<_> = neighbors
            .into_iter()
            .flatten()
            .filter(|neighbor| !visited.contains(&neighbor.key))
            .cloned()
            .collect();

        if remaining_depth == 0 || next_nodes.is_empty() {
            let keys: Vec<_> = current_path.iter().map(|step| step.key.clone()).collect();
            if seen_paths.insert(keys) {
                results.push(self.materialize_graph(direction, current_path));
            }
            return;
        }

        for next in next_nodes {
            visited.insert(next.key.clone());
            current_path.push(GraphTraceStep {
                key: next.key.clone(),
                call_site: Some(next.call_site.clone()),
            });
            self.walk(
                direction,
                remaining_depth - 1,
                current_path,
                visited,
                results,
                seen_paths,
            );
            current_path.pop();
            visited.remove(&next.key);
        }
    }

    fn materialize_graph(
        &self,
        direction: GraphDirection,
        steps: &[GraphTraceStep],
    ) -> CallGraphPath {
        let path: Vec<GraphPathNode> = steps
            .iter()
            .filter_map(|step| self.functions_by_key.get(&step.key))
            .map(FunctionInfo::to_graph_path_node)
            .collect();
        let stacktrace = direction.order_stacktrace(
            path.iter()
                .zip(steps.iter())
                .map(|(node, step)| {
                    let (file, line) = step
                        .call_site
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
}

fn freeze_edges(
    edges: HashMap<FunctionKey, HashMap<FunctionKey, CallSite>>,
) -> HashMap<FunctionKey, Vec<GraphNeighbor>> {
    edges
        .into_iter()
        .map(|(key, values)| {
            let mut values: Vec<_> = values
                .into_iter()
                .map(|(neighbor_key, call_site)| GraphNeighbor {
                    key: neighbor_key,
                    call_site,
                })
                .collect();
            values.sort_by(|left, right| compare_keys(&left.key, &right.key));
            (key, values)
        })
        .collect()
}

fn resolve_enclosing_function(
    functions_by_file_context: &HashMap<(String, String, Option<String>), Vec<FunctionInfo>>,
    file: &str,
    function_name: Option<&str>,
    class_name: Option<&str>,
    line: usize,
) -> Option<FunctionInfo> {
    let function_name = function_name?;
    let mut candidates = functions_by_file_context
        .get(&(
            file.to_string(),
            function_name.to_string(),
            class_name.map(str::to_string),
        ))
        .cloned()
        .unwrap_or_default();

    if candidates.is_empty() && class_name.is_none() {
        candidates.extend(
            functions_by_file_context
                .iter()
                .filter(|((candidate_file, candidate_name, _), _)| {
                    candidate_file == file && candidate_name == function_name
                })
                .flat_map(|(_, functions)| functions.clone()),
        );
    }

    candidates
        .into_iter()
        .find(|function| function.location.start_line <= line && line <= function.location.end_line)
}

fn resolve_call_targets(
    call: &CallInfo,
    caller: &FunctionInfo,
    defs_by_name: &HashMap<String, Vec<FunctionInfo>>,
    fields_by_file_class: &HashMap<(String, String), Vec<FieldInfo>>,
) -> Vec<FunctionKey> {
    let Some(candidates) = defs_by_name.get(&call.callee) else {
        return Vec::new();
    };

    let mut matched = HashSet::new();
    if let Some(object_name) = call.object_name.as_deref() {
        if matches!(object_name, "self" | "this" | "cls") {
            if let Some(class_name) = caller.class_name.as_deref() {
                matched.extend(filter_by_class_name(candidates, class_name));
            }
        } else {
            matched.extend(filter_by_class_name(candidates, object_name));
            if let Some(class_name) = caller.class_name.as_deref() {
                if let Some(attr_name) = extract_instance_attr(object_name) {
                    if let Some(fields) = fields_by_file_class
                        .get(&(caller.location.file.clone(), class_name.to_string()))
                    {
                        for candidate in candidates {
                            for field in fields.iter().filter(|field| field.name == attr_name) {
                                if candidate.class_name.as_deref().is_some_and(|class_name| {
                                    type_matches_class(field.field_type.as_deref(), class_name)
                                }) {
                                    matched.insert(FunctionKey::from_function(candidate));
                                }
                            }
                        }
                    }
                }
            }
            if let Some(param_name) = extract_instance_attr(object_name) {
                for candidate in candidates {
                    for param in caller
                        .params
                        .iter()
                        .filter(|param| param.name == param_name)
                    {
                        if candidate.class_name.as_deref().is_some_and(|class_name| {
                            type_matches_class(param.param_type.as_deref(), class_name)
                        }) {
                            matched.insert(FunctionKey::from_function(candidate));
                        }
                    }
                }
            }
        }
    } else {
        if let Some(class_name) = caller.class_name.as_deref() {
            matched.extend(filter_by_class_name(candidates, class_name));
        }
        let same_file_candidates: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                candidate.class_name.is_none() && candidate.location.file == caller.location.file
            })
            .collect();
        let classless_candidates: Vec<_> = if same_file_candidates.is_empty() {
            candidates
                .iter()
                .filter(|candidate| candidate.class_name.is_none())
                .collect()
        } else {
            same_file_candidates
        };
        for candidate in classless_candidates {
            matched.insert(FunctionKey::from_function(candidate));
        }
    }

    if matched.is_empty() && call.object_name.is_none() {
        matched.extend(candidates.iter().map(FunctionKey::from_function));
    }

    let mut matched: Vec<_> = matched.into_iter().collect();
    matched.sort_by(compare_keys);
    matched
}

fn filter_by_class_name(candidates: &[FunctionInfo], class_name: &str) -> Vec<FunctionKey> {
    candidates
        .iter()
        .filter(|candidate| candidate.class_name.as_deref() == Some(class_name))
        .map(FunctionKey::from_function)
        .collect()
}

fn compare_keys(left: &FunctionKey, right: &FunctionKey) -> std::cmp::Ordering {
    left.file
        .cmp(&right.file)
        .then(left.start_line.cmp(&right.start_line))
        .then(left.end_line.cmp(&right.end_line))
        .then(left.class_name.cmp(&right.class_name))
        .then(left.name.cmp(&right.name))
}

fn compare_graphs(left: &CallGraphPath, right: &CallGraphPath) -> std::cmp::Ordering {
    left.stacktrace
        .cmp(&right.stacktrace)
        .then(left.depth.cmp(&right.depth))
        .then(left.path.len().cmp(&right.path.len()))
}
