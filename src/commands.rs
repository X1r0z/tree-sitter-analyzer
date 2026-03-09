use std::path::Path;

use rayon::prelude::*;
use serde_json::{json, Value};

use crate::analyzers::{SourceAnalyzer, StoreAnalyzer};
use crate::db::{
    db_path_in_current_dir, file_record_metadata, file_record_with_hash, FileIndexData, IndexStore,
    IndexSyncPlan, IndexSynchronizer, IndexedFileRecord,
};
use crate::extractor::CodeExtractor;
use crate::languages::detect_language;
use crate::models::{GraphDirection, IndexInfo};
use crate::output::{self, FunctionView};
use crate::utils::progress_bar;

enum AnalyzerBackend {
    Store(StoreAnalyzer),
    Source(SourceAnalyzer),
}

struct CommandContext {
    real_path: String,
    backend: AnalyzerBackend,
}

impl CommandContext {
    fn load(path: &str, language: Option<&str>) -> anyhow::Result<Self> {
        let real_path = resolve_path(path);
        let backend = if let Some(store_backend) =
            StoreAnalyzer::from_current_dir_if_compatible(&real_path, language)?
        {
            AnalyzerBackend::Store(store_backend)
        } else {
            AnalyzerBackend::Source(SourceAnalyzer::new_with_language(&real_path, language)?)
        };
        Ok(Self { real_path, backend })
    }

    fn searched_files(&self) -> usize {
        match &self.backend {
            AnalyzerBackend::Store(store_backend) => store_backend.file_count(),
            AnalyzerBackend::Source(source_backend) => source_backend.file_count(),
        }
    }
}

pub(crate) fn index(path: &str, language: Option<&str>) -> Value {
    let real_path = resolve_path(path);
    let source_backend = match SourceAnalyzer::new_with_language(&real_path, language) {
        Ok(source_backend) => source_backend,
        Err(error) => return error_response(error),
    };

    let total_files = source_backend.file_count();
    let progress = progress_bar(total_files, "files", "cyan/blue", "Parsing source files");

    let db_path = match db_path_in_current_dir() {
        Ok(path) => path,
        Err(error) => return error_response(error),
    };

    let existing = match IndexStore::open(&db_path) {
        Ok(store)
            if store
                .is_compatible_with(&real_path, language)
                .unwrap_or(false) =>
        {
            store.indexed_files_by_path().unwrap_or_default()
        }
        Ok(_) => Default::default(),
        Err(_) => Default::default(),
    };

    let indexed: Vec<Result<(IndexedFileRecord, Option<FileIndexData>), String>> = source_backend
        .files()
        .par_iter()
        .map(|file| {
            let result = build_file_index(file, existing.get(file));
            progress.inc(1);
            result.map_err(|error| format!("{}: {}", file, error))
        })
        .collect();
    progress.finish_and_clear();

    let mut current_files = Vec::new();
    let mut snapshots = Vec::new();
    let mut errors = Vec::new();
    for entry in indexed {
        match entry {
            Ok((record, snapshot)) => {
                current_files.push(record);
                if let Some(snapshot) = snapshot {
                    snapshots.push(snapshot);
                }
            }
            Err(error) => errors.push(error),
        }
    }

    let plan = IndexSyncPlan {
        current_files,
        changed_snapshots: snapshots,
    };

    let db_progress = progress_bar(0, "steps", "green/blue", "Persisting index data");
    let update_result =
        IndexSynchronizer::sync(&db_path, &real_path, language, &plan, &db_progress);
    db_progress.finish_and_clear();

    if let Err(error) = update_result {
        return error_response(error);
    }

    let index = IndexInfo {
        database: db_path.to_string_lossy().to_string(),
        discovered_files: total_files,
        indexed_files: plan.current_files.len(),
        reparsed_files: plan.changed_snapshots.len(),
        failed_files: errors.len(),
        errors,
    };

    success_response(
        &real_path,
        total_files,
        [(
            "index",
            serde_json::to_value(index).expect("index info should serialize"),
        )],
    )
}

pub(crate) fn functions(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let functions = match &context.backend {
        AnalyzerBackend::Store(store_backend) => match store_backend.find_functions(query) {
            Ok(functions) => functions,
            Err(error) => return error_response(error),
        },
        AnalyzerBackend::Source(source_backend) => source_backend.find_functions(query),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [(
            "functions",
            output::functions(&functions, FunctionView::Summary),
        )],
    )
}

pub(crate) fn classes(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let classes = match &context.backend {
        AnalyzerBackend::Store(store_backend) => match store_backend.find_classes(query) {
            Ok(classes) => classes,
            Err(error) => return error_response(error),
        },
        AnalyzerBackend::Source(source_backend) => source_backend.find_classes(query),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [("classes", output::classes(&classes))],
    )
}

pub(crate) fn fields(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let fields = match &context.backend {
        AnalyzerBackend::Store(store_backend) => match store_backend.find_fields(class_name) {
            Ok(fields) => fields,
            Err(error) => return error_response(error),
        },
        AnalyzerBackend::Source(source_backend) => source_backend.find_fields(class_name),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [
            ("class_name", json!(class_name)),
            ("fields", output::fields(&fields)),
        ],
    )
}

pub(crate) fn imports(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let imports = match &context.backend {
        AnalyzerBackend::Store(store_backend) => match store_backend.find_imports(query) {
            Ok(imports) => imports,
            Err(error) => return error_response(error),
        },
        AnalyzerBackend::Source(source_backend) => source_backend.find_imports(query),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [("imports", output::imports(&imports))],
    )
}

pub(crate) fn annotations(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let annotations = match &context.backend {
        AnalyzerBackend::Store(store_backend) => match store_backend.find_annotations(query) {
            Ok(annotations) => annotations,
            Err(error) => return error_response(error),
        },
        AnalyzerBackend::Source(source_backend) => source_backend.find_annotations(query),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [("annotations", output::annotations(&annotations))],
    )
}

pub(crate) fn callers(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let callers = match &context.backend {
        AnalyzerBackend::Store(store_backend) => {
            match store_backend.find_callers(function_name, class_name) {
                Ok(callers) => callers,
                Err(error) => return error_response(error),
            }
        }
        AnalyzerBackend::Source(source_backend) => {
            source_backend.find_callers(function_name, class_name)
        }
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [
            ("function", json!(function_name)),
            ("class_name", json!(class_name)),
            ("callers", output::callers(&callers)),
        ],
    )
}

pub(crate) fn callees(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let callees = match &context.backend {
        AnalyzerBackend::Store(store_backend) => {
            match store_backend.find_callees(function_name, class_name) {
                Ok(callees) => callees,
                Err(error) => return error_response(error),
            }
        }
        AnalyzerBackend::Source(source_backend) => {
            source_backend.find_callees(function_name, class_name)
        }
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [
            ("function", json!(function_name)),
            ("class_name", json!(class_name)),
            ("callees", output::callees(&callees)),
        ],
    )
}

pub(crate) fn graph(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
    max_depth: usize,
    direction: GraphDirection,
) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let graphs = match &context.backend {
        AnalyzerBackend::Store(store_backend) => {
            match store_backend.find_graphs(function_name, class_name, direction, max_depth) {
                Ok(graphs) => graphs,
                Err(error) => return error_response(error),
            }
        }
        AnalyzerBackend::Source(source_backend) => {
            match source_backend.find_graphs(function_name, class_name, direction, max_depth) {
                Ok(graphs) => graphs,
                Err(error) => return error_response(error),
            }
        }
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [
            ("function", json!(function_name)),
            ("class_name", json!(class_name)),
            ("direction", json!(direction.as_str())),
            ("max_depth", json!(max_depth)),
            ("graphs", json!(graphs)),
        ],
    )
}

pub(crate) fn refs(path: &str, language: Option<&str>, name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    match &context.backend {
        AnalyzerBackend::Store(store_backend) => match store_backend.find_refs(name) {
            Ok(refs) if refs.is_empty() => success_response(
                &context.real_path,
                context.searched_files(),
                [
                    ("name", json!(name)),
                    ("refs", Value::Array(Vec::new())),
                ],
            ),
            Ok(refs) => match SourceAnalyzer::new_with_language(&context.real_path, language) {
                Ok(source_backend) => {
                    let refs = source_backend.hydrate_refs(refs);
                    success_response(
                        &context.real_path,
                        context.searched_files(),
                        [("name", json!(name)), ("refs", output::refs(&refs))],
                    )
                }
                Err(error) => error_response(error),
            },
            Err(error) => error_response(error),
        },
        AnalyzerBackend::Source(source_backend) => {
            let refs = source_backend.find_refs(name);
            success_response(
                &context.real_path,
                context.searched_files(),
                [("name", json!(name)), ("refs", output::refs(&refs))],
            )
        }
    }
}

pub(crate) fn definition(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    match &context.backend {
        AnalyzerBackend::Store(store_backend) => {
            match store_backend.find_functions(function_name) {
                Ok(functions) => {
                    let functions: Vec<_> = functions
                        .into_iter()
                        .filter(|function| {
                            function.name == function_name
                                && (class_name.is_none()
                                    || function.class_name.as_deref() == class_name)
                        })
                        .collect();
                    if functions.is_empty() {
                        return json!({"error": format!("Function '{}' not found", function_name)});
                    }
                    match SourceAnalyzer::new_with_language(&context.real_path, language) {
                        Ok(source_backend) => {
                            let searched_files = functions
                                .iter()
                                .map(|function| function.location.file.as_str())
                                .collect::<std::collections::HashSet<_>>()
                                .len();
                            let functions = source_backend.hydrate_function_bodies(functions);
                            success_response(
                                &context.real_path,
                                searched_files,
                                [
                                    ("class_name", json!(class_name)),
                                    (
                                        "functions",
                                        output::functions(&functions, FunctionView::Definition),
                                    ),
                                ],
                            )
                        }
                        Err(error) => error_response(error),
                    }
                }
                Err(error) => error_response(error),
            }
        }
        AnalyzerBackend::Source(source_backend) => {
            let functions = source_backend.find_function_definitions(function_name, class_name);
            if functions.is_empty() {
                return json!({"error": format!("Function '{}' not found", function_name)});
            }
            success_response(
                &context.real_path,
                context.searched_files(),
                [
                    ("class_name", json!(class_name)),
                    (
                        "functions",
                        output::functions(&functions, FunctionView::Definition),
                    ),
                ],
            )
        }
    }
}

pub(crate) fn super_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let super_classes = match &context.backend {
        AnalyzerBackend::Store(store_backend) => {
            match store_backend.find_super_classes(class_name) {
                Ok(super_classes) => super_classes,
                Err(error) => return error_response(error),
            }
        }
        AnalyzerBackend::Source(source_backend) => source_backend.find_super_classes(class_name),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [
            ("class_name", json!(class_name)),
            ("super_classes", output::classes(&super_classes)),
        ],
    )
}

pub(crate) fn sub_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let sub_classes = match &context.backend {
        AnalyzerBackend::Store(store_backend) => match store_backend.find_sub_classes(class_name) {
            Ok(sub_classes) => sub_classes,
            Err(error) => return error_response(error),
        },
        AnalyzerBackend::Source(source_backend) => source_backend.find_sub_classes(class_name),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [
            ("class_name", json!(class_name)),
            ("sub_classes", output::classes(&sub_classes)),
        ],
    )
}

fn resolve_path(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(path) => path.to_string_lossy().to_string(),
        Err(_) => {
            let path_ref = Path::new(path);
            if path_ref.is_absolute() {
                path.to_string()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path).to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.to_string())
            }
        }
    }
}

fn success_response<const N: usize>(
    path: &str,
    searched_files: usize,
    payload: [(&str, Value); N],
) -> Value {
    let count = payload
        .iter()
        .find_map(|(key, value)| match (key.to_owned(), value) {
            (
                "functions" | "classes" | "fields" | "imports" | "annotations" | "callers"
                | "callees" | "graphs" | "refs" | "super_classes" | "sub_classes",
                Value::Array(items),
            ) => Some(items.len()),
            _ => None,
        })
        .unwrap_or(0);
    let mut map = serde_json::Map::new();
    map.insert("path".into(), json!(path));
    map.insert("searched_files".into(), json!(searched_files));
    map.insert("count".into(), json!(count));
    for (key, value) in payload {
        map.insert(key.into(), value);
    }
    Value::Object(map)
}

fn error_response(error: impl std::fmt::Display) -> Value {
    json!({ "error": error.to_string() })
}

fn build_file_index(
    file: &str,
    existing: Option<&IndexedFileRecord>,
) -> anyhow::Result<(IndexedFileRecord, Option<FileIndexData>)> {
    let language = detect_language(Path::new(file))
        .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file))?
        .to_string();
    let record_without_hash = file_record_metadata(file, &language)?;
    if let Some(record) = existing.filter(|record| {
        record.language == record_without_hash.language
            && record.mtime_nanos == record_without_hash.mtime_nanos
            && record.size_bytes == record_without_hash.size_bytes
    }) {
        return Ok((record.clone(), None));
    }
    let file_record = file_record_with_hash(file, &language)?;

    let mut extractor = CodeExtractor::new(file)?;
    let snapshot = extractor.snapshot_for_index();
    Ok((
        file_record.clone(),
        Some(FileIndexData {
            file: file_record,
            snapshot,
        }),
    ))
}
