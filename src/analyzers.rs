use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::Path;

use rayon::prelude::*;
use rusqlite::Connection;

use crate::db::IndexStore;
use crate::extractor::CodeExtractor;
use crate::graph::{CallGraph, RawPropertyCaller};
use crate::models::*;
use crate::parser::call_targets::has_unique_class_method_target;
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
        let candidate_files = self.search.filter_by_text(function_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let fn_name = function_name.to_string();
        let cn = class_name.map(|s| s.to_string());
        let unique_method_target = cn.as_deref().is_some_and(|target_class_name| {
            has_unique_class_method_target(
                &self.find_function_definitions(&fn_name, None),
                target_class_name,
                |function| {
                    if function.name == fn_name {
                        function.class_name.as_deref()
                    } else {
                        None
                    }
                },
            )
        });
        let mut results: Vec<CallerInfo> =
            self.analyze_files_with_progress(&candidate_files, |f: &String| {
                self.analyze_file(f, |extractor| {
                    extractor.find_function_callers(&fn_name, cn.as_deref(), unique_method_target)
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

        let mut results: Vec<CalleeInfo> =
            self.analyze_files_with_progress(&relevant_files, |f: &String| {
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
        let snapshots = self.analyze_files_with_progress(self.search.files(), |file: &String| {
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
                    object_type: caller.object_type,
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
        let target = self.find_class_by_name(class_name);
        let target = match target {
            Some(class_info) => class_info,
            None => return Vec::new(),
        };

        collect_reachable_bfs(
            [target],
            [class_name.to_string()],
            |current| {
                current
                    .super_classes
                    .iter()
                    .filter_map(|parent_name| {
                        let candidate_files = self.search.filter_by_text(parent_name);
                        let parent_name = parent_name.clone();
                        self.analyze_files_with_progress(&candidate_files, |file: &String| {
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

    pub(crate) fn find_sub_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        collect_reachable_bfs(
            [class_name.to_string()],
            [class_name.to_string()],
            |current_parent| {
                let candidate_files = self.search.filter_by_text(current_parent);
                let current_parent = current_parent.clone();
                self.analyze_files_with_progress(&candidate_files, |file: &String| {
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
        self.analyze_files_with_progress(&candidate_files, |f: &String| {
            self.analyze_file(f, |extractor| extractor.find_class(class_name))
                .flatten()
                .into_iter()
                .collect()
        })
        .into_iter()
        .next()
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::{SourceAnalyzer, StoreAnalyzer};
    use crate::db::{
        file_record_with_hash, FileIndexData, IndexStore, IndexSyncPlan, IndexSynchronizer,
    };
    use crate::extractor::CodeExtractor;
    use crate::models::{CallGraphPath, CalleeInfo, CallerInfo, GraphDirection};
    use crate::utils::progress_bar;

    #[test]
    fn property_assignment_is_not_treated_as_property_call() {
        let dir = write_fixture(
            "test.py",
            r#"class C:
    @property
    def x(self):
        return 1

def f(c: C):
    c.x = 2
    return c.x
"#,
        );

        let source = source_analyzer(dir.path());
        let store = store_analyzer(dir.path());

        let source_callers = source.find_callers("x", Some("C"));
        let store_callers = store.find_callers("x", Some("C")).unwrap();
        assert_eq!(caller_summary(&source_callers), vec![("f".to_string(), 8)]);
        assert_eq!(caller_summary(&store_callers), vec![("f".to_string(), 8)]);

        let source_graphs = source
            .find_graphs("x", Some("C"), GraphDirection::Backward, 3)
            .unwrap();
        let store_graphs = store
            .find_graphs("x", Some("C"), GraphDirection::Backward, 3)
            .unwrap();
        let expected = vec![vec![
            "C.x(test.py:3)".to_string(),
            "f(test.py:8)".to_string(),
        ]];
        assert_eq!(graph_stacktraces(&source_graphs), expected);
        assert_eq!(graph_stacktraces(&store_graphs), expected);
    }

    #[test]
    fn class_descriptor_access_is_not_treated_as_property_call() {
        let dir = write_fixture(
            "test.py",
            r#"class C:
    @property
    def x(self):
        return 1

v = C.x
"#,
        );

        let source = source_analyzer(dir.path());
        let store = store_analyzer(dir.path());

        assert!(source.find_callers("x", Some("C")).is_empty());
        assert!(store.find_callers("x", Some("C")).unwrap().is_empty());

        let expected = vec![vec!["C.x(test.py:3)".to_string()]];
        assert_eq!(
            graph_stacktraces(
                &source
                    .find_graphs("x", Some("C"), GraphDirection::Backward, 2)
                    .unwrap(),
            ),
            expected
        );
        assert_eq!(
            graph_stacktraces(
                &store
                    .find_graphs("x", Some("C"), GraphDirection::Backward, 2)
                    .unwrap(),
            ),
            expected
        );
    }

    #[test]
    fn module_level_instance_property_read_keeps_module_caller() {
        let dir = write_fixture(
            "test.py",
            r#"class C:
    @property
    def x(self):
        return 1

c = C()
v = c.x
"#,
        );

        let source = source_analyzer(dir.path());
        let store = store_analyzer(dir.path());

        let expected_callers = vec![("<module>".to_string(), 7)];
        assert_eq!(
            caller_summary(&source.find_callers("x", Some("C"))),
            expected_callers
        );
        assert_eq!(
            caller_summary(&store.find_callers("x", Some("C")).unwrap()),
            expected_callers
        );

        let expected_graphs = vec![vec![
            "C.x(test.py:3)".to_string(),
            "<module>(test.py:7)".to_string(),
        ]];
        assert_eq!(
            graph_stacktraces(
                &source
                    .find_graphs("x", Some("C"), GraphDirection::Backward, 2)
                    .unwrap(),
            ),
            expected_graphs
        );
        assert_eq!(
            graph_stacktraces(
                &store
                    .find_graphs("x", Some("C"), GraphDirection::Backward, 2)
                    .unwrap(),
            ),
            expected_graphs
        );
    }

    #[test]
    fn indexed_backward_graph_matches_unique_method_fallback_used_by_callers() {
        let dir = write_fixture(
            "test.py",
            r#"class A:
    def foo(self):
        pass

def bar(b):
    return b.foo()
"#,
        );

        let source = source_analyzer(dir.path());
        let store = store_analyzer(dir.path());

        let expected_callers = vec![("bar".to_string(), 6)];
        assert_eq!(
            caller_summary(&source.find_callers("foo", Some("A"))),
            expected_callers
        );
        assert_eq!(
            caller_summary(&store.find_callers("foo", Some("A")).unwrap()),
            expected_callers
        );

        let expected_graphs = vec![vec![
            "A.foo(test.py:2)".to_string(),
            "bar(test.py:6)".to_string(),
        ]];
        assert_eq!(
            graph_stacktraces(
                &source
                    .find_graphs("foo", Some("A"), GraphDirection::Backward, 2)
                    .unwrap(),
            ),
            expected_graphs
        );
        assert_eq!(
            graph_stacktraces(
                &store
                    .find_graphs("foo", Some("A"), GraphDirection::Backward, 2)
                    .unwrap(),
            ),
            expected_graphs
        );
    }

    #[test]
    fn normal_call_matching_is_not_replaced_by_property_matching_in_store_backend() {
        let dir = write_fixture(
            "test.py",
            r#"class A:
    @staticmethod
    def foo():
        pass

class B:
    @staticmethod
    def foo():
        pass

def bar():
    return A.foo()
"#,
        );

        let source = source_analyzer(dir.path());
        let store = store_analyzer(dir.path());

        let expected_a = vec![("bar".to_string(), 12)];
        assert_eq!(
            caller_summary(&source.find_callers("foo", Some("A"))),
            expected_a
        );
        assert_eq!(
            caller_summary(&store.find_callers("foo", Some("A")).unwrap()),
            expected_a
        );
        assert!(source.find_callers("foo", Some("B")).is_empty());
        assert!(store.find_callers("foo", Some("B")).unwrap().is_empty());

        let expected_graphs = vec![vec![
            "A.foo(test.py:3)".to_string(),
            "bar(test.py:12)".to_string(),
        ]];
        assert_eq!(
            graph_stacktraces(
                &store
                    .find_graphs("foo", Some("A"), GraphDirection::Backward, 2)
                    .unwrap(),
            ),
            expected_graphs
        );
    }

    #[test]
    fn source_callers_and_callees_still_return_expected_results() {
        let dir = write_fixture(
            "test.py",
            r#"def alpha():
    pass

def beta():
    pass

def gamma():
    pass

def target():
    alpha()
    beta()

def unrelated():
    gamma()
"#,
        );

        let source = source_analyzer(dir.path());
        assert_eq!(
            caller_summary(&source.find_callers("alpha", None)),
            vec![("target".to_string(), 11)]
        );
        assert_eq!(
            callee_summary(&source.find_callees("target", None)),
            vec![("alpha".to_string(), 11), ("beta".to_string(), 12)]
        );
    }

    fn write_fixture(file_name: &str, content: &str) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(file_name), content).unwrap();
        dir
    }

    fn source_analyzer(root: &Path) -> SourceAnalyzer {
        SourceAnalyzer::new_with_language(root.to_str().unwrap(), Some("python")).unwrap()
    }

    fn store_analyzer(root: &Path) -> StoreAnalyzer {
        let source = source_analyzer(root);
        let mut current_files = Vec::new();
        let mut snapshots = Vec::new();
        for file in source.files() {
            let record = file_record_with_hash(file, "python").unwrap();
            current_files.push(record.clone());
            let mut extractor = CodeExtractor::new(file).unwrap();
            snapshots.push(FileIndexData {
                file: record,
                snapshot: extractor.snapshot_for_index(),
            });
        }

        let plan = IndexSyncPlan {
            current_files,
            changed_snapshots: snapshots,
        };
        let db_path = root.join("tsa.db");
        let progress = progress_bar(0, "steps", "green/blue", "Indexing test fixtures");
        IndexSynchronizer::sync(
            &db_path,
            root.to_str().unwrap(),
            Some("python"),
            &plan,
            &progress,
        )
        .unwrap();
        progress.finish_and_clear();

        let store = IndexStore::open(&db_path).unwrap();
        StoreAnalyzer {
            conn: store.into_connection(),
            language: Some("python".to_string()),
        }
    }

    fn caller_summary(items: &[CallerInfo]) -> Vec<(String, usize)> {
        items
            .iter()
            .map(|item| (item.caller.clone(), item.line))
            .collect()
    }

    fn callee_summary(items: &[CalleeInfo]) -> Vec<(String, usize)> {
        items
            .iter()
            .map(|item| (item.callee.clone(), item.line))
            .collect()
    }

    fn graph_stacktraces(items: &[CallGraphPath]) -> Vec<Vec<String>> {
        items.iter().map(|item| item.stacktrace.clone()).collect()
    }
}
