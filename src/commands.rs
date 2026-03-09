use std::collections::HashSet;

use serde_json::{json, Value};

use crate::backend::{IndexedAnalyzer, SourceAnalyzer};
use crate::index::build_index;
use crate::models::{FunctionInfo, GraphDirection};
use crate::output::{self, FunctionView};

pub fn cmd_functions(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let functions = match &context.backend {
        AnalyzerBackend::Indexed(indexed_backend) => match indexed_backend.find_functions(query) {
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

pub fn cmd_classes(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let classes = match &context.backend {
        AnalyzerBackend::Indexed(indexed_backend) => match indexed_backend.find_classes(query) {
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

pub fn cmd_fields(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let fields = match &context.backend {
        AnalyzerBackend::Indexed(indexed_backend) => {
            match indexed_backend.find_fields(class_name) {
                Ok(fields) => fields,
                Err(error) => return error_response(error),
            }
        }
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

pub fn cmd_imports(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let imports = match &context.backend {
        AnalyzerBackend::Indexed(indexed_backend) => match indexed_backend.find_imports(query) {
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

pub fn cmd_annotations(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let annotations = match &context.backend {
        AnalyzerBackend::Indexed(indexed_backend) => {
            match indexed_backend.find_annotations(query) {
                Ok(annotations) => annotations,
                Err(error) => return error_response(error),
            }
        }
        AnalyzerBackend::Source(source_backend) => source_backend.find_annotations(query),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        [("annotations", output::annotations(&annotations))],
    )
}

pub fn cmd_callers(
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
        AnalyzerBackend::Indexed(indexed_backend) => {
            match indexed_backend.find_callers(function_name, class_name) {
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

pub fn cmd_callees(
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
        AnalyzerBackend::Indexed(indexed_backend) => {
            match indexed_backend.find_callees(function_name, class_name) {
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

pub fn cmd_graph(
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
        AnalyzerBackend::Indexed(indexed_backend) => {
            match indexed_backend.find_graphs(function_name, class_name, direction, max_depth) {
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

pub fn cmd_symbols(path: &str, language: Option<&str>, name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    match &context.backend {
        AnalyzerBackend::Indexed(indexed_backend) => match indexed_backend.find_symbols(name) {
            Ok(refs) if refs.is_empty() => success_response(
                &context.real_path,
                context.searched_files(),
                [
                    ("name", json!(name)),
                    ("references", Value::Array(Vec::new())),
                ],
            ),
            Ok(refs) => match SourceAnalyzer::new_with_language(&context.real_path, language) {
                Ok(source_backend) => {
                    let refs = source_backend.hydrate_symbols(refs);
                    success_response(
                        &context.real_path,
                        context.searched_files(),
                        [
                            ("name", json!(name)),
                            ("references", output::symbol_refs(&refs)),
                        ],
                    )
                }
                Err(error) => error_response(error),
            },
            Err(error) => error_response(error),
        },
        AnalyzerBackend::Source(source_backend) => {
            let refs = source_backend.find_symbols(name);
            success_response(
                &context.real_path,
                context.searched_files(),
                [
                    ("name", json!(name)),
                    ("references", output::symbol_refs(&refs)),
                ],
            )
        }
    }
}

pub fn cmd_definition(
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
        AnalyzerBackend::Indexed(indexed_backend) => {
            match indexed_backend.find_functions(function_name) {
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
                            let searched_files = unique_function_file_count(&functions);
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

pub fn cmd_super_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let super_classes = match &context.backend {
        AnalyzerBackend::Indexed(indexed_backend) => {
            match indexed_backend.find_super_classes(class_name) {
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

pub fn cmd_sub_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let sub_classes = match &context.backend {
        AnalyzerBackend::Indexed(indexed_backend) => {
            match indexed_backend.find_sub_classes(class_name) {
                Ok(sub_classes) => sub_classes,
                Err(error) => return error_response(error),
            }
        }
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

pub fn cmd_index(path: &str, language: Option<&str>) -> Value {
    let real_path = crate::resolve_path(path);
    build_index(&real_path, language)
}

struct CommandContext {
    real_path: String,
    backend: AnalyzerBackend,
}

impl CommandContext {
    fn load(path: &str, language: Option<&str>) -> anyhow::Result<Self> {
        let real_path = crate::resolve_path(path);
        let backend = with_analyzer_backend(&real_path, language)?;
        Ok(Self { real_path, backend })
    }

    fn searched_files(&self) -> usize {
        match &self.backend {
            AnalyzerBackend::Indexed(indexed_backend) => indexed_backend.file_count(),
            AnalyzerBackend::Source(source_backend) => source_backend.file_count(),
        }
    }
}

enum AnalyzerBackend {
    Indexed(IndexedAnalyzer),
    Source(SourceAnalyzer),
}

fn with_analyzer_backend(path: &str, language: Option<&str>) -> anyhow::Result<AnalyzerBackend> {
    if let Some(indexed_backend) = IndexedAnalyzer::from_current_dir_if_compatible(path, language)?
    {
        return Ok(AnalyzerBackend::Indexed(indexed_backend));
    }
    SourceAnalyzer::new_with_language(path, language).map(AnalyzerBackend::Source)
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
                | "callees" | "graphs" | "references" | "super_classes" | "sub_classes",
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

fn unique_function_file_count(functions: &[FunctionInfo]) -> usize {
    functions
        .iter()
        .map(|function| function.location.file.as_str())
        .collect::<HashSet<_>>()
        .len()
}
