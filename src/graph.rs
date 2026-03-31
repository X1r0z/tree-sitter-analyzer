use std::collections::{HashMap, HashSet};
use std::convert::Infallible;

use crate::models::{
    CallGraphPath, CallInfo, ClassInfo, FieldInfo, FunctionInfo, FunctionKey, GraphDirection,
    GraphPathNode, Location, PythonPropertyInfo,
};
use crate::parser::call_targets::{
    matches_module_property_target, matches_property_target as call_matches_property_target,
    resolve_forward_targets_with_fallback, type_matches_class, ForwardTargetContext,
};
use crate::traversal::{collect_paths_dfs, TraversalPathStep};
use crate::utils::select_most_specific_by_line;

#[derive(Debug, Clone)]
pub(crate) struct RawPropertyCaller {
    pub(crate) location: Location,
    pub(crate) property_name: String,
    pub(crate) caller: String,
    pub(crate) caller_class_name: Option<String>,
    pub(crate) object_name: Option<String>,
    pub(crate) object_type: Option<String>,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
struct CallSite {
    file: String,
    line: usize,
}

#[derive(Debug, Clone)]
struct GraphNeighbor {
    key: FunctionKey,
    call_site: CallSite,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
struct GraphEdgeKey {
    neighbor_key: FunctionKey,
    call_site: CallSite,
}

type FunctionsByFileContext = HashMap<(String, String, Option<String>), Vec<FunctionInfo>>;
type FunctionsByFileName = HashMap<(String, String), Vec<FunctionInfo>>;
type ResolutionCacheKey = (String, String, Option<String>, usize);
type DirectSuperClassesByFileClass = HashMap<(String, String), Vec<String>>;

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
        classes: Vec<ClassInfo>,
        fields: Vec<FieldInfo>,
        calls: Vec<CallInfo>,
        python_properties: Vec<PythonPropertyInfo>,
        property_callers: Vec<RawPropertyCaller>,
    ) -> Self {
        let mut functions_by_key = HashMap::new();
        let mut starting_keys = Vec::new();
        let mut defs_by_name: HashMap<String, Vec<FunctionInfo>> = HashMap::new();
        for function in functions {
            let key = FunctionKey::from(&function);
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

        let mut direct_superclasses_by_file_class = HashMap::new();
        for class in classes {
            direct_superclasses_by_file_class
                .insert((class.location.file, class.name), class.super_classes);
        }

        let property_keys: HashSet<(String, Option<String>)> = python_properties
            .into_iter()
            .map(|property| (property.name, property.class_name))
            .collect();

        let mut functions_by_file_context: HashMap<
            (String, String, Option<String>),
            Vec<FunctionInfo>,
        > = HashMap::new();
        let mut functions_by_file_name: HashMap<(String, String), Vec<FunctionInfo>> =
            HashMap::new();
        for function in functions_by_key.values() {
            functions_by_file_context
                .entry((
                    function.location.file.clone(),
                    function.name.clone(),
                    function.class_name.clone(),
                ))
                .or_default()
                .push(function.clone());
            functions_by_file_name
                .entry((function.location.file.clone(), function.name.clone()))
                .or_default()
                .push(function.clone());
        }
        for functions in functions_by_file_context.values_mut() {
            functions
                .sort_by_key(|function| (function.location.start_line, function.location.end_line));
        }
        for functions in functions_by_file_name.values_mut() {
            functions
                .sort_by_key(|function| (function.location.start_line, function.location.end_line));
        }

        let mut forward_edge_sets: HashMap<FunctionKey, HashSet<GraphEdgeKey>> = HashMap::new();
        let mut resolution_cache: HashMap<ResolutionCacheKey, Option<FunctionInfo>> =
            HashMap::new();
        for call in calls {
            let caller = match call.caller.as_deref() {
                None => {
                    let caller =
                        module_caller_function(&call.location.file, call.location.start_line);
                    let module_key = FunctionKey::from(&caller);
                    functions_by_key
                        .entry(module_key)
                        .or_insert_with(|| caller.clone());
                    caller
                }
                Some(caller_name) => {
                    let Some(caller) = resolve_enclosing_function(
                        &functions_by_file_context,
                        &functions_by_file_name,
                        &call.location.file,
                        Some(caller_name),
                        call.caller_class_name.as_deref(),
                        call.location.start_line,
                        &mut resolution_cache,
                    ) else {
                        continue;
                    };
                    caller
                }
            };

            let mut callees = resolve_call_targets(
                &call,
                &caller,
                &defs_by_name,
                &fields_by_file_class,
                &direct_superclasses_by_file_class,
            );
            if callees.is_empty() {
                let unresolved = unresolved_call_key(&call);
                functions_by_key
                    .entry(unresolved.clone())
                    .or_insert_with(|| unresolved_call_function(&call));
                callees.push(unresolved);
            }

            for callee in callees {
                let caller_key = FunctionKey::from(&caller);
                let call_site = CallSite {
                    file: call.location.file.clone(),
                    line: call.location.start_line,
                };
                forward_edge_sets
                    .entry(caller_key)
                    .or_default()
                    .insert(GraphEdgeKey {
                        neighbor_key: callee,
                        call_site,
                    });
            }
        }

        for property_caller in property_callers {
            let module_level = property_caller.caller == "<module>";
            let caller = if module_level {
                let caller = module_caller_function(
                    &property_caller.location.file,
                    property_caller.location.start_line,
                );
                let module_key = FunctionKey::from(&caller);
                functions_by_key
                    .entry(module_key)
                    .or_insert_with(|| caller.clone());
                caller
            } else {
                let Some(caller) = resolve_enclosing_function(
                    &functions_by_file_context,
                    &functions_by_file_name,
                    &property_caller.location.file,
                    Some(&property_caller.caller),
                    property_caller.caller_class_name.as_deref(),
                    property_caller.location.start_line,
                    &mut resolution_cache,
                ) else {
                    continue;
                };
                caller
            };

            let candidate_defs = defs_by_name
                .get(&property_caller.property_name)
                .cloned()
                .unwrap_or_default();
            let mut matched = false;
            for property in candidate_defs {
                if !property_keys.contains(&(property.name.clone(), property.class_name.clone())) {
                    continue;
                }
                let Some(target_class_name) = property.class_name.as_deref() else {
                    continue;
                };
                let matches_target = if module_level {
                    matches_module_property_target(
                        property_caller.object_name.as_deref(),
                        property_caller.object_type.as_deref(),
                        target_class_name,
                    )
                } else {
                    matches_property_target(
                        &caller,
                        property_caller.object_name.as_deref(),
                        Some(target_class_name),
                        &fields_by_file_class,
                    )
                };
                if !matches_target {
                    continue;
                }
                let caller_key = FunctionKey::from(&caller);
                let property_key = FunctionKey::from(&property);
                let call_site = CallSite {
                    file: property_caller.location.file.clone(),
                    line: property_caller.location.start_line,
                };
                forward_edge_sets
                    .entry(caller_key)
                    .or_default()
                    .insert(GraphEdgeKey {
                        neighbor_key: property_key,
                        call_site,
                    });
                matched = true;
            }

            if !matched {
                let unresolved = unresolved_property_key(&property_caller);
                functions_by_key
                    .entry(unresolved.clone())
                    .or_insert_with(|| unresolved_property_function(&property_caller));
                let caller_key = FunctionKey::from(&caller);
                let call_site = CallSite {
                    file: property_caller.location.file.clone(),
                    line: property_caller.location.start_line,
                };
                forward_edge_sets
                    .entry(caller_key)
                    .or_default()
                    .insert(GraphEdgeKey {
                        neighbor_key: unresolved,
                        call_site,
                    });
            }
        }

        let forward_edges = freeze_edges(forward_edge_sets);
        let mut backward_edge_sets: HashMap<FunctionKey, HashSet<GraphEdgeKey>> = HashMap::new();
        for (caller, callees) in &forward_edges {
            for callee in callees {
                backward_edge_sets
                    .entry(callee.key.clone())
                    .or_default()
                    .insert(GraphEdgeKey {
                        neighbor_key: caller.clone(),
                        call_site: callee.call_site.clone(),
                    });
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
        let mut paths = collect_paths_dfs(
            start_nodes,
            direction,
            max_depth,
            |key: &FunctionKey| key.clone(),
            |current| {
                let neighbors = match direction {
                    GraphDirection::Backward => self.backward_edges.get(current),
                    GraphDirection::Forward => self.forward_edges.get(current),
                };
                Ok::<Vec<(FunctionKey, CallSite)>, Infallible>(
                    neighbors
                        .into_iter()
                        .flatten()
                        .cloned()
                        .map(|neighbor| (neighbor.key, neighbor.call_site))
                        .collect(),
                )
            },
            graph_path_identity,
            |direction, steps| self.materialize_graph(direction, steps),
        )
        .unwrap_or_else(|never| match never {});
        paths.sort_by(compare_graphs);
        paths
    }

    fn materialize_graph(
        &self,
        direction: GraphDirection,
        steps: &[TraversalPathStep<FunctionKey, CallSite>],
    ) -> CallGraphPath {
        let path: Vec<GraphPathNode> = steps
            .iter()
            .filter_map(|step| self.functions_by_key.get(&step.node))
            .map(GraphPathNode::from)
            .collect();
        let stacktrace = direction.order_stacktrace(
            path.iter()
                .zip(steps.iter())
                .map(|(node, step)| {
                    let (file, line) = step
                        .edge
                        .as_ref()
                        .map(|call_site| (call_site.file.as_str(), call_site.line))
                        .unwrap_or((node.location.file.as_str(), node.location.start_line));
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

fn matches_property_target(
    caller: &FunctionInfo,
    object_name: Option<&str>,
    class_name: Option<&str>,
    fields_by_file_class: &HashMap<(String, String), Vec<FieldInfo>>,
) -> bool {
    let Some(class_name) = class_name else {
        return false;
    };

    call_matches_property_target(
        caller.class_name.as_deref(),
        object_name,
        class_name,
        |attr_name, target_class_name| {
            caller
                .class_name
                .as_deref()
                .and_then(|caller_class_name| {
                    fields_by_file_class
                        .get(&(caller.location.file.clone(), caller_class_name.to_string()))
                })
                .into_iter()
                .flatten()
                .any(|field| {
                    field.name == attr_name
                        && type_matches_class(field.field_type.as_deref(), target_class_name)
                })
        },
        |attr_name, target_class_name| {
            caller.params.iter().any(|param| {
                param.name == attr_name
                    && type_matches_class(param.param_type.as_deref(), target_class_name)
            })
        },
    )
}

fn freeze_edges(
    edges: HashMap<FunctionKey, HashSet<GraphEdgeKey>>,
) -> HashMap<FunctionKey, Vec<GraphNeighbor>> {
    edges
        .into_iter()
        .map(|(key, values)| {
            let mut values: Vec<_> = values
                .into_iter()
                .map(|edge| GraphNeighbor {
                    key: edge.neighbor_key,
                    call_site: edge.call_site,
                })
                .collect();
            values.sort_by(|left, right| {
                compare_keys(&left.key, &right.key)
                    .then(compare_call_sites(&left.call_site, &right.call_site))
            });
            (key, values)
        })
        .collect()
}

fn graph_path_identity(
    steps: &[TraversalPathStep<FunctionKey, CallSite>],
) -> Vec<(FunctionKey, Option<CallSite>)> {
    steps
        .iter()
        .map(|step| (step.node.clone(), step.edge.clone()))
        .collect()
}

fn resolve_enclosing_function(
    functions_by_file_context: &FunctionsByFileContext,
    functions_by_file_name: &FunctionsByFileName,
    file: &str,
    function_name: Option<&str>,
    class_name: Option<&str>,
    line: usize,
    resolution_cache: &mut HashMap<ResolutionCacheKey, Option<FunctionInfo>>,
) -> Option<FunctionInfo> {
    let function_name = function_name?;
    let cache_key = (
        file.to_string(),
        function_name.to_string(),
        class_name.map(str::to_string),
        line,
    );
    if let Some(cached) = resolution_cache.get(&cache_key) {
        return cached.clone();
    }

    let selected = functions_by_file_context
        .get(&(
            file.to_string(),
            function_name.to_string(),
            class_name.map(str::to_string),
        ))
        .and_then(|candidates| {
            select_most_specific_by_line(candidates, line, |function| {
                (function.location.start_line, function.location.end_line)
            })
        })
        .cloned()
        .or_else(|| {
            if class_name.is_some() {
                return None;
            }
            functions_by_file_name
                .get(&(file.to_string(), function_name.to_string()))
                .and_then(|candidates| {
                    select_most_specific_by_line(candidates, line, |function| {
                        (function.location.start_line, function.location.end_line)
                    })
                })
                .cloned()
        });

    resolution_cache.insert(cache_key, selected.clone());
    selected
}

fn resolve_call_targets(
    call: &CallInfo,
    caller: &FunctionInfo,
    defs_by_name: &HashMap<String, Vec<FunctionInfo>>,
    fields_by_file_class: &HashMap<(String, String), Vec<FieldInfo>>,
    direct_superclasses_by_file_class: &DirectSuperClassesByFileClass,
) -> Vec<FunctionKey> {
    let Some(candidates) = defs_by_name.get(&call.callee) else {
        return Vec::new();
    };
    if call.object_name.as_deref() == Some("super()") {
        let Some(caller_class_name) = caller.class_name.as_deref() else {
            return Vec::new();
        };
        let Some(direct_superclasses) = direct_superclasses_by_file_class
            .get(&(caller.location.file.clone(), caller_class_name.to_string()))
        else {
            return Vec::new();
        };
        let mut matched: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                candidate.class_name.as_deref().is_some_and(|class_name| {
                    direct_superclasses
                        .iter()
                        .any(|parent| parent == class_name)
                })
            })
            .map(FunctionKey::from)
            .collect();
        matched.sort_by(compare_keys);
        matched.dedup();
        return matched;
    }
    let matched: Vec<_> = resolve_forward_targets_with_fallback(
        ForwardTargetContext {
            caller_class_name: caller.class_name.as_deref(),
            caller_file: &caller.location.file,
            object_name: call.object_name.as_deref(),
        },
        candidates,
        |candidate: &FunctionInfo| FunctionKey::from(candidate),
        |candidate| candidate.class_name.as_deref(),
        |candidate| candidate.location.file.as_str(),
        |attr_name, candidate_class_name| {
            caller
                .class_name
                .as_deref()
                .and_then(|class_name| {
                    fields_by_file_class
                        .get(&(caller.location.file.clone(), class_name.to_string()))
                })
                .into_iter()
                .flatten()
                .any(|field| {
                    field.name == attr_name
                        && type_matches_class(field.field_type.as_deref(), candidate_class_name)
                })
        },
        |attr_name, candidate_class_name| {
            caller.params.iter().any(|param| {
                param.name == attr_name
                    && type_matches_class(param.param_type.as_deref(), candidate_class_name)
            })
        },
    )
    .into_iter()
    .map(|candidate| FunctionKey::from(&candidate))
    .collect();
    let mut matched = matched;
    matched.sort_by(compare_keys);
    matched
}

fn compare_keys(left: &FunctionKey, right: &FunctionKey) -> std::cmp::Ordering {
    left.location
        .file
        .cmp(&right.location.file)
        .then(left.location.start_line.cmp(&right.location.start_line))
        .then(left.location.end_line.cmp(&right.location.end_line))
        .then(left.class_name.cmp(&right.class_name))
        .then(left.name.cmp(&right.name))
}

fn compare_call_sites(left: &CallSite, right: &CallSite) -> std::cmp::Ordering {
    left.file.cmp(&right.file).then(left.line.cmp(&right.line))
}

fn compare_graphs(left: &CallGraphPath, right: &CallGraphPath) -> std::cmp::Ordering {
    left.stacktrace
        .cmp(&right.stacktrace)
        .then(left.depth.cmp(&right.depth))
        .then(left.path.len().cmp(&right.path.len()))
}

fn unresolved_call_key(call: &CallInfo) -> FunctionKey {
    let name = unresolved_call_name(call);
    FunctionKey {
        location: Location {
            file: call.location.file.clone(),
            start_line: call.location.start_line,
            end_line: call.location.start_line,
        },
        name: name.clone(),
        class_name: None,
    }
}

fn unresolved_call_function(call: &CallInfo) -> FunctionInfo {
    FunctionInfo {
        name: unresolved_call_name(call),
        location: Location {
            file: call.location.file.clone(),
            start_line: call.location.start_line,
            end_line: call.location.start_line,
        },
        body: String::new(),
        class_name: None,
        params: Vec::new(),
    }
}

fn unresolved_call_name(call: &CallInfo) -> String {
    unresolved_name(call.object_name.as_deref(), &call.callee)
}

fn unresolved_property_key(property_caller: &RawPropertyCaller) -> FunctionKey {
    let name = unresolved_name(
        property_caller.object_name.as_deref(),
        &property_caller.property_name,
    );
    FunctionKey {
        location: Location {
            file: property_caller.location.file.clone(),
            start_line: property_caller.location.start_line,
            end_line: property_caller.location.end_line,
        },
        name: name.clone(),
        class_name: None,
    }
}

fn unresolved_property_function(property_caller: &RawPropertyCaller) -> FunctionInfo {
    FunctionInfo {
        name: unresolved_name(
            property_caller.object_name.as_deref(),
            &property_caller.property_name,
        ),
        location: Location {
            file: property_caller.location.file.clone(),
            start_line: property_caller.location.start_line,
            end_line: property_caller.location.end_line,
        },
        body: String::new(),
        class_name: None,
        params: Vec::new(),
    }
}

fn unresolved_name(object_name: Option<&str>, callee_name: &str) -> String {
    match object_name {
        Some(object_name) => format!("{}.{}", object_name, callee_name),
        None => callee_name.to_string(),
    }
}

fn module_caller_function(file: &str, line: usize) -> FunctionInfo {
    FunctionInfo {
        name: "<module>".to_string(),
        location: Location {
            file: file.to_string(),
            start_line: line,
            end_line: line,
        },
        body: String::new(),
        class_name: None,
        params: Vec::new(),
    }
}
