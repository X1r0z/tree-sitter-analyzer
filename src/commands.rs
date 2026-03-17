use std::path::Path;

use rayon::prelude::*;
use serde_json::{json, Value};

use crate::analyzers::{SourceAnalyzer, StoreAnalyzer};
use crate::db::{
    db_path_in_current_dir, file_record_metadata, file_record_with_hash_from_source, FileIndexData,
    IndexStore, IndexSyncPlan, IndexSynchronizer, IndexedFileRecord,
};
use crate::extractor::CodeExtractor;
use crate::languages::detect_language;
use crate::models::{GraphDirection, IndexInfo};
use crate::output::{self, FunctionView};
use crate::utils::{print_warning, progress_bar};

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

    fn backend_name(&self) -> &'static str {
        match self.backend {
            AnalyzerBackend::Store(_) => "store",
            AnalyzerBackend::Source(_) => "source",
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
            store
                .indexed_files_by_paths(source_backend.files())
                .unwrap_or_default()
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
        changed_files: snapshots,
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
        candidates: total_files,
        indexed: plan.current_files.len(),
        reparsed: plan.changed_files.len(),
        failed: errors.len(),
        errors,
    };

    success_response(&real_path, "source", total_files, output::index(&index))
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
        AnalyzerBackend::Source(source_backend) => match source_backend.find_functions(query) {
            Ok(functions) => functions,
            Err(error) => return error_response(error),
        },
    };
    success_response(
        &context.real_path,
        context.backend_name(),
        context.searched_files(),
        output::functions(&functions, FunctionView::Summary),
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
        AnalyzerBackend::Source(source_backend) => match source_backend.find_classes(query) {
            Ok(classes) => classes,
            Err(error) => return error_response(error),
        },
    };
    success_response(
        &context.real_path,
        context.backend_name(),
        context.searched_files(),
        output::classes(&classes),
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
        context.backend_name(),
        context.searched_files(),
        output::fields(&fields),
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
        AnalyzerBackend::Source(source_backend) => match source_backend.find_imports(query) {
            Ok(imports) => imports,
            Err(error) => return error_response(error),
        },
    };
    success_response(
        &context.real_path,
        context.backend_name(),
        context.searched_files(),
        output::imports(&imports),
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
        AnalyzerBackend::Source(source_backend) => match source_backend.find_annotations(query) {
            Ok(annotations) => annotations,
            Err(error) => return error_response(error),
        },
    };
    success_response(
        &context.real_path,
        context.backend_name(),
        context.searched_files(),
        output::annotations(&annotations),
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
        context.backend_name(),
        context.searched_files(),
        output::callers(&callers),
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
        context.backend_name(),
        context.searched_files(),
        output::callees(&callees),
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
            print_warning(
                "graph on source mode scans all files; run tsa index for repeated queries",
            );
            match source_backend.find_graphs(function_name, class_name, direction, max_depth) {
                Ok(graphs) => graphs,
                Err(error) => return error_response(error),
            }
        }
    };
    success_response(
        &context.real_path,
        context.backend_name(),
        context.searched_files(),
        output::graphs(&graphs),
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
                context.backend_name(),
                context.searched_files(),
                Value::Array(Vec::new()),
            ),
            Ok(refs) => match SourceAnalyzer::new_with_language(&context.real_path, language) {
                Ok(source_backend) => {
                    let refs = source_backend.hydrate_refs(refs);
                    success_response(
                        &context.real_path,
                        context.backend_name(),
                        context.searched_files(),
                        output::refs(&refs),
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
                context.backend_name(),
                context.searched_files(),
                output::refs(&refs),
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
                                context.backend_name(),
                                searched_files,
                                output::functions(&functions, FunctionView::Definition),
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
                context.backend_name(),
                context.searched_files(),
                output::functions(&functions, FunctionView::Definition),
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
        context.backend_name(),
        context.searched_files(),
        output::classes(&super_classes),
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
        context.backend_name(),
        context.searched_files(),
        output::classes(&sub_classes),
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

fn success_response(path: &str, backend: &str, searched_files: usize, results: Value) -> Value {
    let count = match &results {
        Value::Array(items) => items.len(),
        _ => 0,
    };
    json!({
        "meta": {
            "root": path,
            "backend": backend,
            "files": searched_files,
            "count": count,
        },
        "results": results,
    })
}

fn error_response(error: impl std::fmt::Display) -> Value {
    json!({ "error": error.to_string() })
}

fn build_file_index(
    file: &str,
    existing: Option<&IndexedFileRecord>,
) -> anyhow::Result<(IndexedFileRecord, Option<FileIndexData>)> {
    use std::fs;

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
    let source = fs::read(file)?;
    let file_record = file_record_with_hash_from_source(file, &language, &source)?;

    let mut extractor = CodeExtractor::from_source(file, source)?;
    let snapshot = extractor.build_snapshot();
    Ok((
        file_record.clone(),
        Some(FileIndexData {
            file: file_record,
            snapshot,
        }),
    ))
}
