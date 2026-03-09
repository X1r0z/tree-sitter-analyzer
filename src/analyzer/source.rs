use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::Path;

use rayon::prelude::*;

use super::extractor::CodeExtractor;
use super::graph::{CallGraph, RawPropertyCaller};
use super::search::{FileSearch, QueryMatcher};
use crate::models::*;
use crate::traversal::bfs::collect_reachable;
use crate::utils::{
    find_files, progress_bar, sort_callees_by_file_line, sort_callers_by_file_line,
};

pub struct SourceAnalyzer {
    search: FileSearch,
}

impl SourceAnalyzer {
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
            search: FileSearch::new(path, files),
        })
    }

    fn analyze_file<R>(&self, file: &str, f: impl FnOnce(&mut CodeExtractor) -> R) -> Option<R> {
        let mut extractor = CodeExtractor::new(file).ok()?;
        Some(f(&mut extractor))
    }

    fn analyze_files_with_progress<T, R, F>(&self, files: &[T], f: F) -> Vec<R>
    where
        T: Sync,
        F: Fn(&T) -> Vec<R> + Sync + Send,
        R: Send,
    {
        if files.is_empty() {
            return Vec::new();
        }

        let progress = progress_bar(files.len(), "files", "cyan/blue", "Analyzing source files");
        let results = files
            .par_iter()
            .flat_map(|file| {
                let result = f(file);
                progress.inc(1);
                result
            })
            .collect();
        progress.finish_and_clear();
        results
    }

    pub fn find_functions(&self, query: &str) -> Vec<FunctionInfo> {
        self.collect_functions(query)
    }

    pub fn file_count(&self) -> usize {
        self.search.files().len()
    }

    pub fn files(&self) -> &[String] {
        self.search.files()
    }

    fn hydrate_candidates<K, Candidate, KeyOf, FileOf, Resolve>(
        &self,
        candidates: Vec<Candidate>,
        key_of: KeyOf,
        file_of: FileOf,
        resolve: Resolve,
    ) -> Vec<Candidate>
    where
        K: Clone + Eq + Hash + Send + Sync,
        Candidate: Clone + Send + Sync,
        KeyOf: Fn(&Candidate) -> K + Sync,
        FileOf: Fn(&Candidate) -> &str,
        Resolve: Fn(&mut CodeExtractor, &HashSet<K>) -> Vec<Candidate> + Sync + Send,
    {
        if candidates.is_empty() {
            return Vec::new();
        }

        let mut expected_keys_by_file: HashMap<String, HashSet<K>> = HashMap::new();
        for candidate in &candidates {
            expected_keys_by_file
                .entry(file_of(candidate).to_string())
                .or_default()
                .insert(key_of(candidate));
        }

        let expected_keys_by_file: Vec<_> = expected_keys_by_file.into_iter().collect();
        let resolved: HashMap<K, Candidate> = self
            .analyze_files_with_progress(&expected_keys_by_file, |(file, expected)| {
                self.analyze_file(file, |extractor| resolve(extractor, expected))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|candidate| (key_of(&candidate), candidate))
                    .collect::<Vec<_>>()
            })
            .into_iter()
            .collect();

        candidates
            .into_iter()
            .map(|candidate| {
                resolved
                    .get(&key_of(&candidate))
                    .cloned()
                    .unwrap_or(candidate)
            })
            .collect()
    }

    pub fn hydrate_function_bodies(&self, candidates: Vec<FunctionInfo>) -> Vec<FunctionInfo> {
        self.hydrate_candidates(
            candidates,
            |candidate: &FunctionInfo| FunctionKey::from(candidate),
            |candidate| candidate.location.file.as_str(),
            |extractor, expected| {
                extractor
                    .collect_functions_with_bodies()
                    .into_iter()
                    .filter(|function| expected.contains(&FunctionKey::from(function)))
                    .collect()
            },
        )
    }

    fn collect_functions(&self, query: &str) -> Vec<FunctionInfo> {
        let candidate_files = self.search.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        self.analyze_files_with_progress(&candidate_files, |f| {
            let funcs = self
                .analyze_file(f, |extractor| extractor.collect_functions())
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
    }

    pub fn find_classes(&self, query: &str) -> Vec<ClassInfo> {
        let candidate_files = self.search.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        self.analyze_files_with_progress(&candidate_files, |f| {
            let classes = self
                .analyze_file(f, |extractor| extractor.collect_classes())
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
    }

    pub fn find_fields(&self, class_name: &str) -> Vec<FieldInfo> {
        let candidate_files = self.search.filter_by_text(class_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let cn = class_name.to_string();
        self.analyze_files_with_progress(&candidate_files, |f| {
            self.analyze_file(f, |extractor| extractor.collect_fields(&cn))
                .unwrap_or_default()
        })
    }

    pub fn find_imports(&self, query: &str) -> Vec<ImportInfo> {
        let candidate_files = self.search.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        self.analyze_files_with_progress(&candidate_files, |f| {
            let imports = self
                .analyze_file(f, |extractor| extractor.collect_imports())
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
    }
    pub fn find_annotations(&self, query: &str) -> Vec<AnnotationInfo> {
        let candidate_files = self.search.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        self.analyze_files_with_progress(&candidate_files, |f| {
            let annotations = self
                .analyze_file(f, |extractor| extractor.collect_annotations())
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
    }

    pub fn find_callers(&self, function_name: &str, class_name: Option<&str>) -> Vec<CallerInfo> {
        let candidate_files = self.search.filter_by_text(function_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let fn_name = function_name.to_string();
        let cn = class_name.map(|s| s.to_string());
        let mut results: Vec<CallerInfo> =
            self.analyze_files_with_progress(&candidate_files, |f| {
                self.analyze_file(f, |extractor| {
                    extractor.find_function_callers(&fn_name, cn.as_deref())
                })
                .unwrap_or_default()
                .into_iter()
                .map(|(caller, line)| CallerInfo {
                    caller,
                    line,
                    file: f.clone(),
                })
                .collect::<Vec<_>>()
            });
        sort_callers_by_file_line(&mut results);
        results
    }

    pub fn find_callees(&self, function_name: &str, class_name: Option<&str>) -> Vec<CalleeInfo> {
        let candidate_files = self.search.filter_by_text(function_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }

        let fn_name = function_name.to_string();
        let cn = class_name.map(|s| s.to_string());
        let mut relevant_files: Vec<String> =
            self.analyze_files_with_progress(&candidate_files, |f| {
                self.analyze_file(f, |extractor| {
                    extractor.has_function_named(&fn_name, cn.as_deref())
                })
                .and_then(|exists| exists.then(|| f.clone()))
                .into_iter()
                .collect()
            });

        if relevant_files.is_empty() {
            relevant_files = candidate_files;
        }

        let mut results: Vec<CalleeInfo> = self.analyze_files_with_progress(&relevant_files, |f| {
            self.analyze_file(f, |extractor| {
                extractor.find_function_callees(&fn_name, cn.as_deref())
            })
            .unwrap_or_default()
            .into_iter()
            .map(|(callee, line, callee_class)| CalleeInfo {
                callee,
                line,
                file: f.clone(),
                class_name: callee_class,
            })
            .collect::<Vec<_>>()
        });
        sort_callees_by_file_line(&mut results);
        results
    }

    pub fn find_function_definitions(
        &self,
        name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        let candidate_files = self.search.filter_by_text(name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let fn_name = name.to_string();
        let cn = class_name.map(|s| s.to_string());
        self.analyze_files_with_progress(&candidate_files, |f| {
            self.analyze_file(f, |extractor| {
                extractor.find_function_definitions(&fn_name, cn.as_deref())
            })
            .unwrap_or_default()
        })
    }

    pub fn find_graphs(
        &self,
        function_name: &str,
        class_name: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
    ) -> anyhow::Result<Vec<CallGraphPath>> {
        let snapshots = self.analyze_files_with_progress(self.search.files(), |file| {
            vec![match CodeExtractor::new(file) {
                Ok(mut extractor) => anyhow::Ok(extractor.snapshot_for_index()),
                Err(error) => Err(error),
            }]
        });

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
                    caller_class_name: caller.caller_class_name,
                    object_name: caller.object_name,
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

    pub fn find_symbols(&self, name: &str) -> Vec<SymbolRefInfo> {
        let candidate_files = self.search.filter_by_text(name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let symbol = name.to_string();
        let refs: Vec<SymbolRefInfo> = self.analyze_files_with_progress(&candidate_files, |f| {
            self.analyze_file(f, |extractor| extractor.find_symbols(&symbol))
                .unwrap_or_default()
        });
        refs
    }

    pub fn hydrate_symbols(&self, candidates: Vec<SymbolRefInfo>) -> Vec<SymbolRefInfo> {
        self.hydrate_candidates(
            candidates,
            |candidate: &SymbolRefInfo| SymbolRefKey::from(candidate),
            |candidate| candidate.location.file.as_str(),
            |extractor, expected| {
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
                extractor.hydrate_symbols(&expected_symbols)
            },
        )
    }

    pub fn find_super_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        let target = self.find_class_by_name(class_name);
        let target = match target {
            Some(c) => c,
            None => return Vec::new(),
        };

        collect_reachable(
            [target],
            [class_name.to_string()],
            |current| {
                current
                    .super_classes
                    .iter()
                    .filter_map(|parent_name| {
                        let candidate_files = self.search.filter_by_text(parent_name);
                        let parent_name = parent_name.clone();
                        self.analyze_files_with_progress(&candidate_files, |file| {
                            self.analyze_file(file, |extractor| extractor.find_class(&parent_name))
                                .flatten()
                                .into_iter()
                                .collect::<Vec<_>>()
                        })
                        .into_iter()
                        .next()
                    })
                    .collect()
            },
            |parent| parent.clone(),
            |parent| parent.name.clone(),
        )
    }

    pub fn find_sub_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        collect_reachable(
            [class_name.to_string()],
            [class_name.to_string()],
            |current_parent| {
                let candidate_files = self.search.filter_by_text(current_parent);
                let current_parent = current_parent.clone();
                self.analyze_files_with_progress(&candidate_files, |file| {
                    self.analyze_file(file, |extractor| extractor.collect_classes())
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|class| class.super_classes.contains(&current_parent))
                        .collect::<Vec<_>>()
                })
            },
            |child| child.name.clone(),
            |child| child.name.clone(),
        )
    }

    fn find_class_by_name(&self, class_name: &str) -> Option<ClassInfo> {
        let candidate_files = self.search.filter_by_text(class_name);
        self.analyze_files_with_progress(&candidate_files, |f| {
            self.analyze_file(f, |extractor| extractor.find_class(class_name))
                .flatten()
                .into_iter()
                .collect()
        })
        .into_iter()
        .next()
    }
}
