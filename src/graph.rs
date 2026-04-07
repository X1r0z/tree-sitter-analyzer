use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::path::Path;

use crate::models::{
    CallGraphPath, CallInfo, ClassInfo, FieldInfo, FunctionInfo, FunctionKey, GraphDirection,
    GraphPathNode, Location, PythonPropertyInfo,
};
use crate::parser::call_targets::{
    matches_module_property_target, matches_property_target as call_matches_property_target,
    resolve_forward_targets_with_fallback, type_matches_class, ForwardTargetContext,
};
use crate::traversal::{collect_paths_dfs, collect_reachable_bfs, TraversalPathStep};
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

type DefinitionsByName = HashMap<String, Vec<FunctionKey>>;
type FunctionsByFileContext = HashMap<(String, String, Option<String>), Vec<FunctionKey>>;
type FunctionsByFileName = HashMap<(String, String), Vec<FunctionKey>>;
type EnclosingResolutionCacheKey = (String, String, Option<String>, usize);
type CallTargetResolutionCacheKey = (FunctionKey, String, Option<String>);
type PropertyTargetResolutionCacheKey = (FunctionKey, String, Option<String>, Option<String>, bool);
type ParentsByFileClass = HashMap<(String, String), Vec<String>>;

#[derive(Debug, Clone)]
pub(crate) struct CallGraph {
    functions_by_key: HashMap<FunctionKey, FunctionInfo>,
    starting_keys_by_name: HashMap<String, Vec<FunctionKey>>,
    forward_edges: HashMap<FunctionKey, Vec<GraphNeighbor>>,
    backward_edges: HashMap<FunctionKey, Vec<GraphNeighbor>>,
    file_display_names: HashMap<String, String>,
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
        let mut starting_keys_by_name: HashMap<String, Vec<FunctionKey>> = HashMap::new();
        let mut defs_by_name: DefinitionsByName = HashMap::new();
        for function in functions {
            let key = FunctionKey::from(&function);
            starting_keys_by_name
                .entry(function.name.clone())
                .or_default()
                .push(key.clone());
            defs_by_name
                .entry(function.name.clone())
                .or_default()
                .push(key.clone());
            functions_by_key.insert(key, function);
        }
        for keys in starting_keys_by_name.values_mut() {
            keys.sort_by(compare_keys);
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

        let mut parents_by_file_class = HashMap::new();
        for class in classes {
            parents_by_file_class.insert((class.location.file, class.name), class.super_classes);
        }

        let property_keys: HashSet<(String, Option<String>)> = python_properties
            .into_iter()
            .map(|property| (property.name, property.class_name))
            .collect();

        let mut functions_by_file_context: FunctionsByFileContext = HashMap::new();
        let mut functions_by_file_name: FunctionsByFileName = HashMap::new();
        for (key, function) in &functions_by_key {
            functions_by_file_context
                .entry((
                    function.location.file.clone(),
                    function.name.clone(),
                    function.class_name.clone(),
                ))
                .or_default()
                .push(key.clone());
            functions_by_file_name
                .entry((function.location.file.clone(), function.name.clone()))
                .or_default()
                .push(key.clone());
        }
        for functions in functions_by_file_context.values_mut() {
            functions.sort_by(compare_keys);
        }
        for functions in functions_by_file_name.values_mut() {
            functions.sort_by(compare_keys);
        }

        let mut forward_edge_sets: HashMap<FunctionKey, Vec<GraphNeighbor>> = HashMap::new();
        let mut enclosing_resolution_cache: HashMap<
            EnclosingResolutionCacheKey,
            Option<FunctionKey>,
        > = HashMap::new();
        let mut call_target_resolution_cache: HashMap<
            CallTargetResolutionCacheKey,
            Vec<FunctionKey>,
        > = HashMap::new();
        let mut property_target_resolution_cache: HashMap<
            PropertyTargetResolutionCacheKey,
            Vec<FunctionKey>,
        > = HashMap::new();
        for call in calls {
            let caller_key = match call.caller.as_deref() {
                None => {
                    let caller = FunctionInfo {
                        name: "<module>".to_string(),
                        location: Location {
                            file: call.location.file.clone(),
                            start_line: call.location.start_line,
                            end_line: call.location.start_line,
                        },
                        body: String::new(),
                        class_name: None,
                        params: Vec::new(),
                    };
                    let module_key = FunctionKey::from(&caller);
                    functions_by_key
                        .entry(module_key.clone())
                        .or_insert_with(|| caller.clone());
                    module_key
                }
                Some(caller_name) => {
                    let Some(caller_key) = resolve_enclosing_function(
                        &functions_by_file_context,
                        &functions_by_file_name,
                        &call.location.file,
                        Some(caller_name),
                        call.caller_class_name.as_deref(),
                        call.location.start_line,
                        &mut enclosing_resolution_cache,
                    ) else {
                        continue;
                    };
                    caller_key
                }
            };
            let Some(_caller) = functions_by_key.get(&caller_key) else {
                continue;
            };

            let mut callees = resolve_call_targets(
                &call,
                &caller_key,
                &functions_by_key,
                &defs_by_name,
                &fields_by_file_class,
                &parents_by_file_class,
                &mut call_target_resolution_cache,
            );
            if callees.is_empty() {
                let unresolved_name = unresolved_name(call.object_name.as_deref(), &call.callee);
                let unresolved = FunctionKey {
                    location: Location {
                        file: call.location.file.clone(),
                        start_line: call.location.start_line,
                        end_line: call.location.start_line,
                    },
                    name: unresolved_name.clone(),
                    class_name: None,
                };
                functions_by_key
                    .entry(unresolved.clone())
                    .or_insert_with(|| FunctionInfo {
                        name: unresolved_name.clone(),
                        location: Location {
                            file: call.location.file.clone(),
                            start_line: call.location.start_line,
                            end_line: call.location.start_line,
                        },
                        body: String::new(),
                        class_name: None,
                        params: Vec::new(),
                    });
                callees.push(unresolved);
            }

            for callee in callees {
                let call_site = CallSite {
                    file: call.location.file.clone(),
                    line: call.location.start_line,
                };
                forward_edge_sets
                    .entry(caller_key.clone())
                    .or_default()
                    .push(GraphNeighbor {
                        key: callee,
                        call_site,
                    });
            }
        }

        for property_caller in property_callers {
            let module_level = property_caller.caller == "<module>";
            let caller_key = if module_level {
                let caller = FunctionInfo {
                    name: "<module>".to_string(),
                    location: Location {
                        file: property_caller.location.file.clone(),
                        start_line: property_caller.location.start_line,
                        end_line: property_caller.location.start_line,
                    },
                    body: String::new(),
                    class_name: None,
                    params: Vec::new(),
                };
                let module_key = FunctionKey::from(&caller);
                functions_by_key
                    .entry(module_key.clone())
                    .or_insert_with(|| caller.clone());
                module_key
            } else {
                let Some(caller_key) = resolve_enclosing_function(
                    &functions_by_file_context,
                    &functions_by_file_name,
                    &property_caller.location.file,
                    Some(&property_caller.caller),
                    property_caller.caller_class_name.as_deref(),
                    property_caller.location.start_line,
                    &mut enclosing_resolution_cache,
                ) else {
                    continue;
                };
                caller_key
            };
            let Some(caller) = functions_by_key.get(&caller_key) else {
                continue;
            };

            let candidate_defs = resolve_property_targets(
                caller_key.clone(),
                caller,
                &property_caller,
                module_level,
                &functions_by_key,
                &defs_by_name,
                &property_keys,
                &fields_by_file_class,
                &parents_by_file_class,
                &mut property_target_resolution_cache,
            );
            let mut matched = false;
            for property_key in candidate_defs {
                let call_site = CallSite {
                    file: property_caller.location.file.clone(),
                    line: property_caller.location.start_line,
                };
                forward_edge_sets
                    .entry(caller_key.clone())
                    .or_default()
                    .push(GraphNeighbor {
                        key: property_key,
                        call_site,
                    });
                matched = true;
            }

            if !matched {
                let unresolved_name = unresolved_name(
                    property_caller.object_name.as_deref(),
                    &property_caller.property_name,
                );
                let unresolved = FunctionKey {
                    location: Location {
                        file: property_caller.location.file.clone(),
                        start_line: property_caller.location.start_line,
                        end_line: property_caller.location.start_line,
                    },
                    name: unresolved_name.clone(),
                    class_name: None,
                };
                functions_by_key
                    .entry(unresolved.clone())
                    .or_insert_with(|| FunctionInfo {
                        name: unresolved_name.clone(),
                        location: Location {
                            file: property_caller.location.file.clone(),
                            start_line: property_caller.location.start_line,
                            end_line: property_caller.location.start_line,
                        },
                        body: String::new(),
                        class_name: None,
                        params: Vec::new(),
                    });
                let call_site = CallSite {
                    file: property_caller.location.file.clone(),
                    line: property_caller.location.start_line,
                };
                forward_edge_sets
                    .entry(caller_key.clone())
                    .or_default()
                    .push(GraphNeighbor {
                        key: unresolved,
                        call_site,
                    });
            }
        }

        let forward_edges = freeze_edges(forward_edge_sets);
        let mut backward_edge_sets: HashMap<FunctionKey, Vec<GraphNeighbor>> = HashMap::new();
        for (caller, callees) in &forward_edges {
            for callee in callees {
                backward_edge_sets
                    .entry(callee.key.clone())
                    .or_default()
                    .push(GraphNeighbor {
                        key: caller.clone(),
                        call_site: callee.call_site.clone(),
                    });
            }
        }

        let file_display_names = collect_file_display_names(&functions_by_key);

        Self {
            functions_by_key,
            starting_keys_by_name,
            forward_edges,
            backward_edges: freeze_edges(backward_edge_sets),
            file_display_names,
        }
    }

    pub(crate) fn find_start_nodes(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionKey> {
        let Some(candidates) = self.starting_keys_by_name.get(function_name) else {
            return Vec::new();
        };
        let mut matches: Vec<_> = candidates
            .iter()
            .filter_map(|key| {
                self.functions_by_key.get(key).and_then(|function| {
                    (class_name.is_none() || function.class_name.as_deref() == class_name)
                        .then(|| key.clone())
                })
            })
            .collect();
        if matches.len() > 1 {
            matches.sort_by(compare_keys);
        }
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
                Ok::<Vec<(FunctionKey, CallSite)>, Infallible>(
                    self.collect_neighbor_pairs(current, direction),
                )
            },
            |steps: &[TraversalPathStep<FunctionKey, CallSite>]| {
                steps
                    .iter()
                    .map(|step| (step.node.clone(), step.edge.clone()))
                    .collect::<Vec<_>>()
            },
            |direction, steps| self.materialize_graph(direction, steps),
        )
        .unwrap_or_else(|never| match never {});
        if paths.len() > 1 {
            paths.sort_by(compare_graphs);
        }
        paths
    }

    fn materialize_graph(
        &self,
        direction: GraphDirection,
        steps: &[TraversalPathStep<FunctionKey, CallSite>],
    ) -> CallGraphPath {
        let mut path = Vec::with_capacity(steps.len());
        let mut stacktrace = Vec::with_capacity(steps.len());
        for step in steps {
            let Some(function) = self.functions_by_key.get(&step.node) else {
                continue;
            };
            let node = GraphPathNode::from(function);
            let (file, line) = step
                .edge
                .as_ref()
                .map(|call_site| (call_site.file.as_str(), call_site.line))
                .unwrap_or((node.location.file.as_str(), node.location.start_line));
            stacktrace.push(self.stacktrace_name(&node, file, line));
            path.push(node);
        }
        let stacktrace = direction.order_stacktrace(stacktrace);
        CallGraphPath {
            depth: path.len().saturating_sub(1),
            stacktrace,
            path,
        }
    }

    fn collect_neighbor_pairs(
        &self,
        current: &FunctionKey,
        direction: GraphDirection,
    ) -> Vec<(FunctionKey, CallSite)> {
        let neighbors = match direction {
            GraphDirection::Backward => self.backward_edges.get(current),
            GraphDirection::Forward => self.forward_edges.get(current),
        };
        let Some(neighbors) = neighbors else {
            return Vec::new();
        };
        let mut pairs = Vec::with_capacity(neighbors.len());
        for neighbor in neighbors {
            pairs.push((neighbor.key.clone(), neighbor.call_site.clone()));
        }
        pairs
    }

    fn stacktrace_name(&self, node: &GraphPathNode, file: &str, line: usize) -> String {
        let filename = self
            .file_display_names
            .get(file)
            .map(String::as_str)
            .unwrap_or(file);
        format!("{}({}:{})", node.display_name(), filename, line)
    }
}

fn collect_file_display_names(
    functions_by_key: &HashMap<FunctionKey, FunctionInfo>,
) -> HashMap<String, String> {
    let mut file_display_names = HashMap::new();
    for key in functions_by_key.keys() {
        file_display_names
            .entry(key.location.file.clone())
            .or_insert_with(|| {
                Path::new(&key.location.file)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(&key.location.file)
                    .to_string()
            });
    }
    file_display_names
}

fn matches_property_target(
    caller: &FunctionInfo,
    object_name: Option<&str>,
    class_name: Option<&str>,
    fields_by_file_class: &HashMap<(String, String), Vec<FieldInfo>>,
    parents_by_file_class: &ParentsByFileClass,
) -> bool {
    let Some(class_name) = class_name else {
        return false;
    };
    if matches!(object_name, Some("self") | Some("cls")) {
        let Some(caller_class_name) = caller.class_name.as_deref() else {
            return false;
        };
        return caller_class_name == class_name
            || collect_reachable_bfs(
                [caller_class_name.to_string()],
                [caller_class_name.to_string()],
                |current| {
                    parents_by_file_class
                        .get(&(caller.location.file.clone(), current.to_string()))
                        .cloned()
                        .unwrap_or_default()
                },
                Clone::clone,
                Clone::clone,
            )
            .into_iter()
            .any(|ancestor| ancestor == class_name);
    }

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
    edges: HashMap<FunctionKey, Vec<GraphNeighbor>>,
) -> HashMap<FunctionKey, Vec<GraphNeighbor>> {
    edges
        .into_iter()
        .map(|(key, values)| {
            let mut values = values;
            values.sort_by(|left, right| {
                compare_keys(&left.key, &right.key)
                    .then(compare_call_sites(&left.call_site, &right.call_site))
            });
            values
                .dedup_by(|left, right| left.key == right.key && left.call_site == right.call_site);
            (key, values)
        })
        .collect()
}

fn resolve_enclosing_function(
    functions_by_file_context: &FunctionsByFileContext,
    functions_by_file_name: &FunctionsByFileName,
    file: &str,
    function_name: Option<&str>,
    class_name: Option<&str>,
    line: usize,
    resolution_cache: &mut HashMap<EnclosingResolutionCacheKey, Option<FunctionKey>>,
) -> Option<FunctionKey> {
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
            select_most_specific_by_line(candidates, line, |key| {
                (key.location.start_line, key.location.end_line)
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
                    select_most_specific_by_line(candidates, line, |key| {
                        (key.location.start_line, key.location.end_line)
                    })
                })
                .cloned()
        });

    resolution_cache.insert(cache_key, selected.clone());
    selected
}

fn resolve_call_targets(
    call: &CallInfo,
    caller_key: &FunctionKey,
    functions_by_key: &HashMap<FunctionKey, FunctionInfo>,
    defs_by_name: &DefinitionsByName,
    fields_by_file_class: &HashMap<(String, String), Vec<FieldInfo>>,
    parents_by_file_class: &ParentsByFileClass,
    resolution_cache: &mut HashMap<CallTargetResolutionCacheKey, Vec<FunctionKey>>,
) -> Vec<FunctionKey> {
    let cache_key = (
        caller_key.clone(),
        call.callee.clone(),
        call.object_name.clone(),
    );
    if let Some(cached) = resolution_cache.get(&cache_key) {
        return cached.clone();
    }
    let Some(caller) = functions_by_key.get(caller_key) else {
        return Vec::new();
    };

    let Some(candidates) = defs_by_name.get(&call.callee) else {
        return Vec::new();
    };
    if call.object_name.as_deref() == Some("super()") {
        let Some(caller_class_name) = caller.class_name.as_deref() else {
            return Vec::new();
        };
        let Some(parents) = parents_by_file_class
            .get(&(caller.location.file.clone(), caller_class_name.to_string()))
        else {
            return Vec::new();
        };
        let mut matched: Vec<_> = candidates
            .iter()
            .filter_map(|candidate_key| functions_by_key.get(candidate_key))
            .filter(|candidate| {
                candidate
                    .class_name
                    .as_deref()
                    .is_some_and(|class_name| parents.iter().any(|parent| parent == class_name))
            })
            .map(FunctionKey::from)
            .collect();
        matched.sort_by(compare_keys);
        matched.dedup();
        resolution_cache.insert(cache_key, matched.clone());
        return matched;
    }
    let candidate_refs = candidates
        .iter()
        .filter_map(|candidate_key| functions_by_key.get(candidate_key))
        .collect::<Vec<_>>();
    let matched: Vec<_> = resolve_forward_targets_with_fallback(
        ForwardTargetContext {
            caller_class_name: caller.class_name.as_deref(),
            caller_file: &caller.location.file,
            object_name: call.object_name.as_deref(),
        },
        &candidate_refs,
        |candidate: &&FunctionInfo| FunctionKey::from(*candidate),
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
    .map(FunctionKey::from)
    .collect();
    let mut matched = matched;
    matched.sort_by(compare_keys);
    matched.dedup();
    resolution_cache.insert(cache_key, matched.clone());
    matched
}

fn resolve_property_targets(
    caller_key: FunctionKey,
    caller: &FunctionInfo,
    property_caller: &RawPropertyCaller,
    module_level: bool,
    functions_by_key: &HashMap<FunctionKey, FunctionInfo>,
    defs_by_name: &DefinitionsByName,
    property_keys: &HashSet<(String, Option<String>)>,
    fields_by_file_class: &HashMap<(String, String), Vec<FieldInfo>>,
    parents_by_file_class: &ParentsByFileClass,
    resolution_cache: &mut HashMap<PropertyTargetResolutionCacheKey, Vec<FunctionKey>>,
) -> Vec<FunctionKey> {
    let cache_key = (
        caller_key,
        property_caller.property_name.clone(),
        property_caller.object_name.clone(),
        property_caller.object_type.clone(),
        module_level,
    );
    if let Some(cached) = resolution_cache.get(&cache_key) {
        return cached.clone();
    }

    let Some(candidate_defs) = defs_by_name.get(&property_caller.property_name) else {
        return Vec::new();
    };

    let mut matched_keys = Vec::new();
    for property_key in candidate_defs {
        let Some(property) = functions_by_key.get(property_key) else {
            continue;
        };
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
                caller,
                property_caller.object_name.as_deref(),
                Some(target_class_name),
                fields_by_file_class,
                parents_by_file_class,
            )
        };
        if matches_target {
            matched_keys.push(property_key.clone());
        }
    }

    matched_keys.sort_by(compare_keys);
    matched_keys.dedup();
    if !matched_keys.is_empty() {
        resolution_cache.insert(cache_key, matched_keys.clone());
    }
    matched_keys
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

fn unresolved_name(object_name: Option<&str>, callee_name: &str) -> String {
    match object_name {
        Some(object_name) => format!("{}.{}", object_name, callee_name),
        None => callee_name.to_string(),
    }
}
