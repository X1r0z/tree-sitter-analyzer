use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::Path;

use rayon::prelude::*;
use rusqlite::Connection;

use crate::db::IndexStore;
use crate::extractor::CodeExtractor;
use crate::graph::{CallGraph, RawPropertyCaller};
use crate::models::*;
use crate::parser::call_targets::{has_unique_class_method_target, split_function_target};
use crate::query::{CallEdgeQuery, CallGraphQuery, ClassHierarchyQuery, LookupQuery, QueryContext};
use crate::search::{FileSearch, QueryMatcher};
use crate::traversal::collect_reachable_bfs;
use crate::utils::{
    find_files, progress_bar, sort_callees_by_file_line, sort_callers_by_file_line,
};

pub(crate) struct SourceAnalyzer {
    search: FileSearch,
}

pub(crate) struct StoreAnalyzer {
    conn: Connection,
    language: Option<String>,
}

#[derive(Clone)]
struct CallerFileResult {
    file: String,
    callers: Vec<(String, Location)>,
}

struct GraphFileResult {
    file: String,
    definitions: Vec<FunctionInfo>,
    snapshot: FileSnapshot,
}

#[derive(Default)]
struct ClassHierarchyCache {
    parsed_files: HashMap<String, Vec<ClassInfo>>,
    classes_by_name: HashMap<String, Vec<ClassInfo>>,
    class_keys_by_name: HashMap<String, HashSet<ClassInstanceKey>>,
    direct_subclasses_by_parent: HashMap<String, Vec<ClassInfo>>,
    direct_subclass_keys_by_parent: HashMap<String, HashSet<ClassInstanceKey>>,
    candidate_files_by_name: HashMap<String, Vec<String>>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct ClassInstanceKey {
    file: String,
    name: String,
    start_line: usize,
    end_line: usize,
}

impl SourceAnalyzer {
    pub(crate) fn new_with_language(path: &str, language: Option<&str>) -> anyhow::Result<Self> {
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

    pub(crate) fn file_count(&self) -> usize {
        self.search.files().len()
    }

    pub(crate) fn files(&self) -> &[String] {
        self.search.files()
    }

    pub(crate) fn find_functions(&self, query: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let candidate_files = self.search.filter_candidates(query);
        if candidate_files.is_empty() {
            return Ok(Vec::new());
        }
        let matcher = QueryMatcher::new(query)?;
        Ok(
            self.analyze_files_with_progress(&candidate_files, |f: &String| {
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
            }),
        )
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

    pub(crate) fn hydrate_function_bodies(
        &self,
        candidates: Vec<FunctionInfo>,
    ) -> Vec<FunctionInfo> {
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

    pub(crate) fn find_classes(&self, query: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let candidate_files = self.search.filter_candidates(query);
        if candidate_files.is_empty() {
            return Ok(Vec::new());
        }
        let matcher = QueryMatcher::new(query)?;
        Ok(
            self.analyze_files_with_progress(&candidate_files, |f: &String| {
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
            }),
        )
    }

    pub(crate) fn find_fields(&self, class_name: &str) -> Vec<FieldInfo> {
        let candidate_files = self.search.filter_by_text(class_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let cn = class_name.to_string();
        self.analyze_files_with_progress(&candidate_files, |f: &String| {
            self.analyze_file(f, |extractor| extractor.collect_fields(&cn))
                .unwrap_or_default()
        })
    }

    pub(crate) fn find_imports(&self, query: &str) -> anyhow::Result<Vec<ImportInfo>> {
        let candidate_files = self.search.filter_candidates(query);
        if candidate_files.is_empty() {
            return Ok(Vec::new());
        }
        let matcher = QueryMatcher::new(query)?;
        Ok(
            self.analyze_files_with_progress(&candidate_files, |f: &String| {
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
            }),
        )
    }

    pub(crate) fn find_annotations(&self, query: &str) -> anyhow::Result<Vec<AnnotationInfo>> {
        let candidate_files = self.search.filter_candidates(query);
        if candidate_files.is_empty() {
            return Ok(Vec::new());
        }
        let matcher = QueryMatcher::new(query)?;
        Ok(
            self.analyze_files_with_progress(&candidate_files, |f: &String| {
                let annotations = self
                    .analyze_file(f, |extractor| extractor.collect_annotations())
                    .unwrap_or_default();
                if matcher.matches_all() {
                    annotations
                } else {
                    annotations
                        .into_iter()
                        .filter(|annotation| matcher.is_match(&annotation.name))
                        .collect()
                }
            }),
        )
    }

    pub(crate) fn find_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<CallerInfo> {
        let target_function = split_function_target(function_name).0.to_string();
        let mut caller_candidate_files = self.search.filter_by_text(function_name);
        let definition_candidate_files = self.search.filter_by_text(&target_function);
        caller_candidate_files.sort();
        caller_candidate_files.dedup();
        if caller_candidate_files.is_empty() && definition_candidate_files.is_empty() {
            return Vec::new();
        }

        let unique_method_target = class_name.is_some_and(|target_class_name| {
            let definitions = self.collect_function_definition_results(
                &definition_candidate_files,
                &target_function,
                None,
            );
            has_unique_class_method_target(&definitions, target_class_name, |function| {
                if function.name == target_function {
                    function.class_name.as_deref()
                } else {
                    None
                }
            })
        });
        let mut callers: Vec<CallerInfo> = self
            .collect_caller_file_results(
                &caller_candidate_files,
                function_name,
                class_name,
                unique_method_target,
            )
            .into_iter()
            .flat_map(|result| {
                result
                    .callers
                    .into_iter()
                    .map(move |(caller, location)| CallerInfo {
                        caller,
                        location: Location {
                            file: result.file.clone(),
                            start_line: location.start_line,
                            end_line: location.end_line,
                        },
                    })
            })
            .collect();
        let mut seen = HashSet::new();
        callers.retain(|caller| {
            seen.insert((
                caller.location.file.clone(),
                caller.caller.clone(),
                caller.location.start_line,
                caller.location.end_line,
            ))
        });
        sort_callers_by_file_line(&mut callers);
        callers
    }

    pub(crate) fn find_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<CalleeInfo> {
        let candidate_files = self.search.filter_by_text(function_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }

        let fn_name = function_name.to_string();
        let cn = class_name.map(|s| s.to_string());
        let mut results: Vec<CalleeInfo> =
            self.analyze_files_with_progress(&candidate_files, |f: &String| {
                self.analyze_file(f, |extractor| {
                    extractor.find_function_callees_if_present(&fn_name, cn.as_deref())
                })
                .unwrap_or_default()
                .into_iter()
                .map(|(callee, location)| CalleeInfo { callee, location })
                .collect::<Vec<_>>()
            });
        sort_callees_by_file_line(&mut results);
        results
    }

    pub(crate) fn find_function_definitions(
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
        self.analyze_files_with_progress(&candidate_files, |f: &String| {
            self.analyze_file(f, |extractor| {
                extractor.find_function_definitions(&fn_name, cn.as_deref())
            })
            .unwrap_or_default()
        })
    }

    pub(crate) fn find_graphs(
        &self,
        function_name: &str,
        class_name: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
    ) -> anyhow::Result<Vec<CallGraphPath>> {
        let snapshots = match direction {
            GraphDirection::Forward => self
                .collect_forward_graph_results(function_name, class_name)
                .into_iter()
                .map(|result| result.map(|entry| entry.snapshot))
                .collect::<Vec<_>>(),
            GraphDirection::Backward => self
                .collect_backward_graph_results(function_name, class_name)?
                .into_iter()
                .map(Ok)
                .collect::<Vec<_>>(),
        };

        let mut functions = Vec::new();
        let mut classes = Vec::new();
        let mut fields = Vec::new();
        let mut calls = Vec::new();
        let mut python_properties = Vec::new();
        let mut property_callers = Vec::new();

        for snapshot in snapshots {
            let snapshot = snapshot?;
            functions.extend(snapshot.functions);
            classes.extend(snapshot.classes);
            fields.extend(snapshot.fields);
            calls.extend(snapshot.calls);
            python_properties.extend(snapshot.python_properties);
            property_callers.extend(snapshot.python_property_callers.into_iter().map(|caller| {
                RawPropertyCaller {
                    location: caller.location,
                    property_name: caller.property_name,
                    caller: caller.caller,
                    caller_class_name: caller.caller_class_name,
                    object_name: caller.object_name,
                    object_type: caller.object_type,
                }
            }));
        }

        let graph = CallGraph::build(
            functions,
            classes,
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

    pub(crate) fn find_refs(&self, name: &str) -> Vec<RefInfo> {
        let candidate_files = self.search.filter_by_text(name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let reference_name = name.to_string();
        self.analyze_files_with_progress(&candidate_files, |f: &String| {
            self.analyze_file(f, |extractor| extractor.find_refs(&reference_name))
                .unwrap_or_default()
        })
    }

    pub(crate) fn hydrate_refs(&self, candidates: Vec<RefInfo>) -> Vec<RefInfo> {
        self.hydrate_candidates(
            candidates,
            |candidate: &RefInfo| RefKey::from(candidate),
            |candidate| candidate.location.file.as_str(),
            |extractor, expected| {
                let expected_refs: Vec<_> = expected
                    .iter()
                    .map(|key| RefInfo {
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
                extractor.hydrate_refs(&expected_refs)
            },
        )
    }

    pub(crate) fn find_super_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        let mut hierarchy = ClassHierarchyCache::default();
        self.ensure_files_parsed_for_name(&mut hierarchy, class_name);
        let targets = hierarchy.find_classes_by_name(class_name);
        if targets.is_empty() {
            return Vec::new();
        }

        collect_reachable_bfs(
            targets.clone(),
            targets.iter().map(|class_info| ClassInstanceKey {
                file: class_info.location.file.clone(),
                name: class_info.name.clone(),
                start_line: class_info.location.start_line,
                end_line: class_info.location.end_line,
            }),
            |current| {
                current
                    .super_classes
                    .iter()
                    .flat_map(|parent_name| {
                        self.ensure_files_parsed_for_name(&mut hierarchy, parent_name);
                        hierarchy.find_classes_by_name(parent_name)
                    })
                    .collect()
            },
            |parent| parent.clone(),
            |class_info| ClassInstanceKey {
                file: class_info.location.file.clone(),
                name: class_info.name.clone(),
                start_line: class_info.location.start_line,
                end_line: class_info.location.end_line,
            },
        )
    }

    pub(crate) fn find_sub_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        let mut hierarchy = ClassHierarchyCache::default();
        self.ensure_files_parsed_for_name(&mut hierarchy, class_name);
        let targets = hierarchy.find_classes_by_name(class_name);
        if targets.is_empty() {
            return Vec::new();
        }

        collect_reachable_bfs(
            targets.clone(),
            targets.iter().map(|class_info| ClassInstanceKey {
                file: class_info.location.file.clone(),
                name: class_info.name.clone(),
                start_line: class_info.location.start_line,
                end_line: class_info.location.end_line,
            }),
            |current| {
                self.ensure_files_parsed_for_name(&mut hierarchy, &current.name);
                hierarchy.find_direct_subclasses(&current.name)
            },
            |child| child.clone(),
            |class_info| ClassInstanceKey {
                file: class_info.location.file.clone(),
                name: class_info.name.clone(),
                start_line: class_info.location.start_line,
                end_line: class_info.location.end_line,
            },
        )
    }

    fn collect_caller_file_results(
        &self,
        caller_candidate_files: &[String],
        function_name: &str,
        class_name: Option<&str>,
        allow_unique_method_target: bool,
    ) -> Vec<CallerFileResult> {
        let function_name = function_name.to_string();
        let class_name = class_name.map(str::to_string);
        self.analyze_files_with_progress(caller_candidate_files, |file: &String| {
            self.analyze_file(file, |extractor| {
                vec![CallerFileResult {
                    file: file.clone(),
                    callers: extractor.find_function_callers(
                        &function_name,
                        class_name.as_deref(),
                        allow_unique_method_target,
                    ),
                }]
            })
            .unwrap_or_default()
        })
    }

    fn collect_function_definition_results(
        &self,
        definition_candidate_files: &[String],
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        let function_name = function_name.to_string();
        let class_name = class_name.map(str::to_string);
        self.analyze_files_with_progress(definition_candidate_files, |file: &String| {
            self.analyze_file(file, |extractor| {
                extractor.find_function_signatures(&function_name, class_name.as_deref())
            })
            .unwrap_or_default()
        })
    }

    fn collect_graph_file_results(
        &self,
        files: &[String],
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<anyhow::Result<GraphFileResult>> {
        let fn_name = function_name.to_string();
        let class_name = class_name.map(str::to_string);
        self.analyze_files_with_progress(files, |file: &String| {
            vec![match CodeExtractor::new(file) {
                Ok(mut extractor) => Ok(GraphFileResult {
                    file: file.clone(),
                    definitions: extractor
                        .find_function_signatures(&fn_name, class_name.as_deref()),
                    snapshot: extractor.build_snapshot(),
                }),
                Err(error) => Err(error),
            }]
        })
    }

    fn collect_forward_graph_results(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<anyhow::Result<GraphFileResult>> {
        let candidate_files = self.search.files().to_vec();
        self.collect_graph_file_results(&candidate_files, function_name, class_name)
    }

    fn collect_backward_graph_results(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<FileSnapshot>> {
        let base_function_name = split_function_target(function_name).0.to_string();
        let initial_candidate_files = self.search.filter_by_text(&base_function_name);
        let initial_results = self.collect_graph_file_results(
            &initial_candidate_files,
            &base_function_name,
            class_name,
        );

        let mut reused_snapshots = HashMap::new();
        let mut definition_files = Vec::new();
        for result in initial_results {
            let result = result?;
            definition_files.extend(
                result
                    .definitions
                    .iter()
                    .map(|function| function.location.file.clone()),
            );
            reused_snapshots.insert(result.file, result.snapshot);
        }

        let mut candidate_files = initial_candidate_files;
        candidate_files.extend(definition_files);
        if let Some(class_name) = class_name {
            candidate_files.extend(self.search.filter_by_text(class_name));
        }
        candidate_files.sort();
        candidate_files.dedup();
        if candidate_files.is_empty() && !self.search.files().is_empty() {
            candidate_files = self.search.files().to_vec();
        }

        let extra_files: Vec<_> = candidate_files
            .iter()
            .filter(|file| !reused_snapshots.contains_key(file.as_str()))
            .cloned()
            .collect();
        let extra_results =
            self.collect_graph_file_results(&extra_files, &base_function_name, class_name);

        let mut snapshots = Vec::new();
        for file in candidate_files {
            if let Some(snapshot) = reused_snapshots.remove(&file) {
                snapshots.push(snapshot);
            }
        }
        for result in extra_results {
            snapshots.push(result?.snapshot);
        }

        Ok(snapshots)
    }

    fn ensure_files_parsed_for_name(&self, cache: &mut ClassHierarchyCache, class_name: &str) {
        let new_files = cache.unparsed_candidate_files_for_name(class_name, &self.search);
        if new_files.is_empty() {
            return;
        }

        let parsed_results = self.analyze_files_with_progress(&new_files, |file: &String| {
            self.analyze_file(file, |extractor| {
                vec![(file.clone(), extractor.collect_classes())]
            })
            .unwrap_or_default()
        });
        for (file, classes) in parsed_results {
            cache.insert_file_classes(file, classes);
        }
    }
}

impl ClassHierarchyCache {
    fn unparsed_candidate_files_for_name(
        &mut self,
        class_name: &str,
        search: &FileSearch,
    ) -> Vec<String> {
        let candidate_files = self
            .candidate_files_by_name
            .entry(class_name.to_string())
            .or_insert_with(|| search.filter_by_text(class_name));

        candidate_files
            .iter()
            .filter(|file| !self.parsed_files.contains_key(file.as_str()))
            .cloned()
            .collect()
    }

    fn insert_file_classes(&mut self, file: String, classes: Vec<ClassInfo>) {
        let mut deduped_classes = Vec::new();
        let mut file_seen = HashSet::new();
        for class_info in classes {
            let key = ClassInstanceKey::from(&class_info);
            if file_seen.insert(key) {
                deduped_classes.push(class_info);
            }
        }

        for class_info in &deduped_classes {
            let key = ClassInstanceKey::from(class_info);
            if self
                .class_keys_by_name
                .entry(class_info.name.clone())
                .or_default()
                .insert(key.clone())
            {
                self.classes_by_name
                    .entry(class_info.name.clone())
                    .or_default()
                    .push(class_info.clone());
            }

            for parent_name in &class_info.super_classes {
                if self
                    .direct_subclass_keys_by_parent
                    .entry(parent_name.clone())
                    .or_default()
                    .insert(key.clone())
                {
                    self.direct_subclasses_by_parent
                        .entry(parent_name.clone())
                        .or_default()
                        .push(class_info.clone());
                }
            }
        }

        self.parsed_files.insert(file, deduped_classes);
    }

    fn find_classes_by_name(&self, class_name: &str) -> Vec<ClassInfo> {
        self.classes_by_name
            .get(class_name)
            .cloned()
            .unwrap_or_default()
    }

    fn find_direct_subclasses(&self, class_name: &str) -> Vec<ClassInfo> {
        self.direct_subclasses_by_parent
            .get(class_name)
            .cloned()
            .unwrap_or_default()
    }
}

impl From<&ClassInfo> for ClassInstanceKey {
    fn from(class_info: &ClassInfo) -> Self {
        Self {
            file: class_info.location.file.clone(),
            name: class_info.name.clone(),
            start_line: class_info.location.start_line,
            end_line: class_info.location.end_line,
        }
    }
}

impl StoreAnalyzer {
    pub(crate) fn from_current_dir_if_compatible(
        root_path: &str,
        language: Option<&str>,
    ) -> anyhow::Result<Option<Self>> {
        let db_path = std::env::current_dir()?.join("tsa.db");
        if !db_path.exists() {
            return Ok(None);
        }
        let store = match IndexStore::open_if_compatible(&db_path, root_path, language)? {
            Some(store) => store,
            None => return Ok(None),
        };
        Ok(Some(Self {
            conn: store.into_connection(),
            language: language.map(str::to_string),
        }))
    }

    pub(crate) fn file_count(&self) -> usize {
        LookupQuery::new(self.query_context()).file_count()
    }

    pub(crate) fn find_functions(&self, query: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        LookupQuery::new(self.query_context()).find_functions(query)
    }

    pub(crate) fn find_classes(&self, query: &str) -> anyhow::Result<Vec<ClassInfo>> {
        LookupQuery::new(self.query_context()).find_classes(query)
    }

    pub(crate) fn find_fields(&self, class_name: &str) -> anyhow::Result<Vec<FieldInfo>> {
        LookupQuery::new(self.query_context()).find_fields(class_name)
    }

    pub(crate) fn find_imports(&self, query: &str) -> anyhow::Result<Vec<ImportInfo>> {
        LookupQuery::new(self.query_context()).find_imports(query)
    }

    pub(crate) fn find_annotations(&self, query: &str) -> anyhow::Result<Vec<AnnotationInfo>> {
        LookupQuery::new(self.query_context()).find_annotations(query)
    }

    pub(crate) fn find_refs(&self, name: &str) -> anyhow::Result<Vec<RefInfo>> {
        LookupQuery::new(self.query_context()).find_refs(name)
    }

    pub(crate) fn find_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CallerInfo>> {
        CallEdgeQuery::new(self.query_context()).find_callers(function_name, class_name)
    }

    pub(crate) fn find_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CalleeInfo>> {
        CallEdgeQuery::new(self.query_context()).find_callees(function_name, class_name)
    }

    pub(crate) fn find_graphs(
        &self,
        function_name: &str,
        class_name: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
    ) -> anyhow::Result<Vec<CallGraphPath>> {
        CallGraphQuery::new(self.query_context()).find_graphs(
            function_name,
            class_name,
            direction,
            max_depth,
        )
    }

    pub(crate) fn find_super_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        ClassHierarchyQuery::new(self.query_context()).find_super_classes(class_name)
    }

    pub(crate) fn find_sub_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        ClassHierarchyQuery::new(self.query_context()).find_sub_classes(class_name)
    }

    fn query_context(&self) -> QueryContext<'_> {
        QueryContext {
            conn: &self.conn,
            language: self.language.as_deref(),
        }
    }
}
