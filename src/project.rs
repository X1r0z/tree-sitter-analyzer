use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use rayon::prelude::*;

use crate::analyzer::CodeAnalyzer;
use crate::cache::TextFilterCache;
use crate::graph::{CallGraph, RawPropertyCaller};
use crate::nodes::*;
use crate::utils::{
    find_files, is_simple_query, search_files_with_rg, sort_by_file_line, QueryMatcher,
};

pub struct ProjectAnalyzer {
    pub files: Vec<String>,
    path: String,
    text_filter_cache: TextFilterCache,
}

impl ProjectAnalyzer {
    pub fn new_with_language(path: &str, language: Option<&str>) -> anyhow::Result<Self> {
        let p = Path::new(path);
        if !p.exists() {
            anyhow::bail!("Path not found: {}", path);
        }
        if !p.is_dir() {
            anyhow::bail!("Path must be a directory: {}", path);
        }
        let files = find_files(path, language);
        Ok(Self {
            files,
            path: path.to_string(),
            text_filter_cache: TextFilterCache::new(),
        })
    }

    fn analyze_file<R>(&self, file: &str, f: impl FnOnce(&mut CodeAnalyzer) -> R) -> Option<R> {
        let mut analyzer = CodeAnalyzer::new(file).ok()?;
        Some(f(&mut analyzer))
    }

    pub fn find_functions(&self, query: &str) -> Vec<FunctionInfo> {
        self.collect_functions(query)
    }

    pub fn hydrate_function_bodies(&self, candidates: Vec<FunctionInfo>) -> Vec<FunctionInfo> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let mut candidate_keys_by_file: HashMap<String, HashSet<FunctionKey>> = HashMap::new();
        for candidate in &candidates {
            candidate_keys_by_file
                .entry(candidate.location.file.clone())
                .or_default()
                .insert(FunctionKey::from_function(candidate));
        }

        let resolved: HashMap<FunctionKey, FunctionInfo> = candidate_keys_by_file
            .par_iter()
            .flat_map(|(file, expected)| {
                self.analyze_file(file, |analyzer| analyzer.functions_with_bodies())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|function| {
                        let key = FunctionKey::from_function(&function);
                        expected.contains(&key).then_some((key, function))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        candidates
            .into_iter()
            .filter_map(|candidate| {
                resolved
                    .get(&FunctionKey::from_function(&candidate))
                    .cloned()
                    .or(Some(candidate))
            })
            .collect()
    }

    fn collect_functions(&self, query: &str) -> Vec<FunctionInfo> {
        let candidate_files = self.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let funcs = self
                    .analyze_file(f, |analyzer| analyzer.functions())
                    .unwrap_or_default();
                if matcher.matches_all() {
                    funcs
                } else {
                    funcs
                        .into_iter()
                        .filter(|func| matcher.is_match(&func.name))
                        .collect()
                }
            })
            .collect()
    }

    pub fn find_classes(&self, query: &str) -> Vec<ClassInfo> {
        let candidate_files = self.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let classes = self
                    .analyze_file(f, |analyzer| analyzer.classes())
                    .unwrap_or_default();
                if matcher.matches_all() {
                    classes
                } else {
                    classes
                        .into_iter()
                        .filter(|class| matcher.is_match(&class.name))
                        .collect()
                }
            })
            .collect()
    }

    pub fn find_fields(&self, class_name: &str) -> Vec<FieldInfo> {
        let candidate_files = self.filter_by_text(class_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let cn = class_name.to_string();
        candidate_files
            .par_iter()
            .flat_map(|f| {
                self.analyze_file(f, |analyzer| analyzer.fields(&cn))
                    .unwrap_or_default()
            })
            .collect()
    }

    pub fn find_imports(&self, query: &str) -> Vec<ImportInfo> {
        let candidate_files = self.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let imports = self
                    .analyze_file(f, |analyzer| analyzer.imports())
                    .unwrap_or_default();
                if matcher.matches_all() {
                    imports
                } else {
                    imports
                        .into_iter()
                        .filter(|import| matcher.is_match(&import.module))
                        .collect()
                }
            })
            .collect()
    }
    pub fn find_annotations(&self, query: &str) -> Vec<AnnotationInfo> {
        let candidate_files = self.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let annotations = self
                    .analyze_file(f, |analyzer| analyzer.annotations())
                    .unwrap_or_default();
                if matcher.matches_all() {
                    annotations
                } else {
                    annotations
                        .into_iter()
                        .filter(|a| matcher.is_match(&a.name))
                        .collect()
                }
            })
            .collect()
    }

    pub fn find_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let candidate_files = self.filter_by_text(function_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let fn_name = function_name.to_string();
        let cn = class_name.map(|s| s.to_string());
        let mut results: Vec<serde_json::Value> = candidate_files
            .par_iter()
            .flat_map(|f| {
                self.analyze_file(f, |analyzer| {
                    analyzer.find_function_callers(&fn_name, cn.as_deref())
                })
                .unwrap_or_default()
                .into_iter()
                .map(|(caller, line)| {
                    serde_json::json!({
                        "caller": caller,
                        "line": line,
                        "file": f,
                        "target_class": cn.as_deref(),
                    })
                })
                .collect::<Vec<_>>()
            })
            .collect();
        sort_by_file_line(&mut results);
        results
    }

    pub fn find_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let candidate_files = self.filter_by_text(function_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }

        let fn_name = function_name.to_string();
        let cn = class_name.map(|s| s.to_string());
        let mut relevant_files: Vec<String> = candidate_files
            .par_iter()
            .filter_map(|f| {
                self.analyze_file(f, |analyzer| {
                    analyzer.has_function_named(&fn_name, cn.as_deref())
                })
                .and_then(|exists| exists.then(|| f.clone()))
            })
            .collect();

        if relevant_files.is_empty() {
            relevant_files = candidate_files;
        }

        let mut results: Vec<serde_json::Value> = relevant_files
            .par_iter()
            .flat_map(|f| {
                self.analyze_file(f, |analyzer| {
                    analyzer.find_function_callees(&fn_name, cn.as_deref())
                })
                .unwrap_or_default()
                .into_iter()
                .map(|(callee, line, callee_class)| {
                    serde_json::json!({
                            "callee": callee,
                        "line": line,
                        "file": f,
                        "class_name": callee_class,
                    })
                })
                .collect::<Vec<_>>()
            })
            .collect();
        sort_by_file_line(&mut results);
        results
    }

    pub fn find_function_definitions(
        &self,
        name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        let candidate_files = self.filter_by_text(name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let fn_name = name.to_string();
        let cn = class_name.map(|s| s.to_string());
        candidate_files
            .par_iter()
            .flat_map(|f| {
                self.analyze_file(f, |analyzer| {
                    analyzer.find_function_definitions(&fn_name, cn.as_deref())
                })
                .unwrap_or_default()
            })
            .collect()
    }

    pub fn find_graphs(
        &self,
        function_name: &str,
        class_name: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
    ) -> anyhow::Result<Vec<CallGraphPath>> {
        let snapshots = self
            .files
            .par_iter()
            .map(|file| {
                let mut analyzer = CodeAnalyzer::new(file)?;
                anyhow::Ok(analyzer.snapshot_for_index())
            })
            .collect::<Vec<_>>();

        let mut functions = Vec::new();
        let mut fields = Vec::new();
        let mut calls = Vec::new();
        let mut python_properties = Vec::new();
        let mut property_callers = Vec::new();

        for snapshot in snapshots {
            let snapshot = snapshot?;
            functions.extend(snapshot.functions);
            fields.extend(snapshot.fields);
            calls.extend(snapshot.calls);
            python_properties.extend(snapshot.python_properties);
            property_callers.extend(snapshot.python_property_callers.into_iter().map(|caller| {
                RawPropertyCaller {
                    file: caller.file,
                    property_name: caller.property_name,
                    caller: caller.caller,
                    line: caller.line,
                }
            }));
        }

        let graph = CallGraph::build(
            functions,
            fields,
            calls,
            python_properties,
            property_callers,
        );
        let start_nodes = graph.find_start_nodes(function_name, class_name);
        if start_nodes.is_empty() {
            anyhow::bail!("Function '{}' not found", function_name);
        }

        Ok(graph.collect_graphs(&start_nodes, direction, max_depth))
    }

    pub fn find_symbols(&self, name: &str) -> Vec<serde_json::Value> {
        let candidate_files = self.filter_by_text(name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let symbol = name.to_string();
        let refs: Vec<SymbolRefInfo> = candidate_files
            .par_iter()
            .flat_map(|f| {
                self.analyze_file(f, |analyzer| analyzer.find_symbols(&symbol))
                    .unwrap_or_default()
            })
            .collect();
        refs.into_iter()
            .map(|symbol| symbol.to_json_value())
            .collect()
    }

    pub fn hydrate_symbol_contexts(&self, candidates: Vec<SymbolRefInfo>) -> Vec<SymbolRefInfo> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let mut candidate_keys_by_file: HashMap<String, HashSet<SymbolRefKey>> = HashMap::new();
        for candidate in &candidates {
            candidate_keys_by_file
                .entry(candidate.location.file.clone())
                .or_default()
                .insert(SymbolRefKey::from_symbol(candidate));
        }

        let resolved: HashMap<SymbolRefKey, SymbolRefInfo> = candidate_keys_by_file
            .par_iter()
            .flat_map(|(file, expected)| {
                self.analyze_file(file, |analyzer| {
                    let expected_symbols: Vec<_> = expected
                        .iter()
                        .map(|key| SymbolRefInfo {
                            name: key.name.clone(),
                            node_type: key.node_type.clone(),
                            location: Location {
                                file: key.file.clone(),
                                start_line: key.start_line,
                                end_line: key.end_line,
                            },
                            start_column: key.start_column,
                            end_column: key.end_column,
                            context: String::new(),
                        })
                        .collect();
                    analyzer.hydrate_symbol_contexts(&expected_symbols)
                })
                .unwrap_or_default()
                .into_iter()
                .map(|symbol| (SymbolRefKey::from_symbol(&symbol), symbol))
                .collect::<Vec<_>>()
            })
            .collect();

        candidates
            .into_iter()
            .filter_map(|candidate| {
                resolved
                    .get(&SymbolRefKey::from_symbol(&candidate))
                    .cloned()
                    .or(Some(candidate))
            })
            .collect()
    }

    pub fn find_super_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        let target = self.find_class_by_name(class_name);
        let target = match target {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut result = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(class_name.to_string());
        let mut queue: VecDeque<ClassInfo> = VecDeque::new();
        queue.push_back(target);

        while let Some(current) = queue.pop_front() {
            for parent_name in &current.super_classes {
                if visited.contains(parent_name) {
                    continue;
                }
                visited.insert(parent_name.clone());
                let candidate_files = self.filter_by_text(parent_name);
                let pn = parent_name.clone();
                let found: Vec<ClassInfo> = candidate_files
                    .par_iter()
                    .flat_map(|f| {
                        self.analyze_file(f, |analyzer| analyzer.class_named(&pn))
                            .flatten()
                            .into_iter()
                            .collect::<Vec<_>>()
                    })
                    .collect();
                if let Some(parent) = found.into_iter().next() {
                    result.push(parent.clone());
                    queue.push_back(parent);
                }
            }
        }
        result
    }

    pub fn find_sub_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        let mut result = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(class_name.to_string());
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(class_name.to_string());

        while let Some(current_parent) = queue.pop_front() {
            let candidate_files = self.filter_by_text(&current_parent);
            let cp = current_parent.clone();
            let found: Vec<ClassInfo> = candidate_files
                .par_iter()
                .flat_map(|f| {
                    self.analyze_file(f, |analyzer| analyzer.classes())
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|cls| cls.super_classes.contains(&cp))
                        .collect::<Vec<_>>()
                })
                .collect();
            for cls in found {
                if visited.insert(cls.name.clone()) {
                    queue.push_back(cls.name.clone());
                    result.push(cls);
                }
            }
        }
        result
    }

    fn find_class_by_name(&self, class_name: &str) -> Option<ClassInfo> {
        let candidate_files = self.filter_by_text(class_name);
        for f in &candidate_files {
            if let Some(cls) = self
                .analyze_file(f, |analyzer| analyzer.class_named(class_name))
                .flatten()
            {
                return Some(cls);
            }
        }
        None
    }

    fn filter_candidates(&self, query: &str) -> Vec<String> {
        if !query.is_empty() && is_simple_query(query) {
            self.filter_by_text(query)
        } else {
            self.files.clone()
        }
    }

    fn filter_by_text(&self, text: &str) -> Vec<String> {
        if text.is_empty() {
            return self.files.clone();
        }
        if let Some(cached) = self.text_filter_cache.get(text) {
            return cached;
        }
        let matched_files: Vec<String> = if let Some(rg_files) =
            search_files_with_rg(text, &self.path, None)
        {
            let mut rg_files = rg_files;
            rg_files.sort();
            rg_files.dedup();

            let mut matched = Vec::new();
            let mut files_idx = 0usize;
            let mut rg_idx = 0usize;
            while files_idx < self.files.len() && rg_idx < rg_files.len() {
                match self.files[files_idx].cmp(&rg_files[rg_idx]) {
                    std::cmp::Ordering::Less => files_idx += 1,
                    std::cmp::Ordering::Greater => rg_idx += 1,
                    std::cmp::Ordering::Equal => {
                        matched.push(self.files[files_idx].clone());
                        files_idx += 1;
                        rg_idx += 1;
                    }
                }
            }
            matched
        } else {
            self.files
                .par_iter()
                .filter(|f| {
                    std::fs::read(f)
                        .map(|content| content.windows(text.len()).any(|w| w == text.as_bytes()))
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        };
        self.text_filter_cache.insert(text, matched_files.clone());
        matched_files
    }
}
