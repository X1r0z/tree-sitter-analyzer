use std::collections::HashSet;

use serde_json::{json, Value};

use crate::db::DbProjectAnalyzer;
use crate::index::build_index;
use crate::models::{FunctionInfo, GraphDirection};
use crate::project::ProjectAnalyzer;

pub(crate) fn cmd_functions(path: &str, language: Option<&str>, query: &str) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_functions(query) {
            Ok(functions) => success_response(
                &real_path,
                db.file_count(),
                [("functions", functions_to_json(&functions, false))],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let functions = project.find_functions(query);
            success_response(
                &real_path,
                project.file_count(),
                [("functions", functions_to_json(&functions, false))],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_classes(path: &str, language: Option<&str>, query: &str) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_classes(query) {
            Ok(classes) => success_response(
                &real_path,
                db.file_count(),
                [(
                    "classes",
                    Value::Array(
                        classes
                            .iter()
                            .map(|class| class.to_json_value(true))
                            .collect(),
                    ),
                )],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let classes = project.find_classes(query);
            success_response(
                &real_path,
                project.file_count(),
                [(
                    "classes",
                    Value::Array(
                        classes
                            .iter()
                            .map(|class| class.to_json_value(true))
                            .collect(),
                    ),
                )],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_fields(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_fields(class_name) {
            Ok(fields) => success_response(
                &real_path,
                db.file_count(),
                [
                    ("class_name", json!(class_name)),
                    (
                        "fields",
                        Value::Array(
                            fields
                                .iter()
                                .map(|field| field.to_json_value(true))
                                .collect(),
                        ),
                    ),
                ],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let fields = project.find_fields(class_name);
            success_response(
                &real_path,
                project.file_count(),
                [
                    ("class_name", json!(class_name)),
                    (
                        "fields",
                        Value::Array(
                            fields
                                .iter()
                                .map(|field| field.to_json_value(true))
                                .collect(),
                        ),
                    ),
                ],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_imports(path: &str, language: Option<&str>, query: &str) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_imports(query) {
            Ok(imports) => success_response(
                &real_path,
                db.file_count(),
                [(
                    "imports",
                    Value::Array(
                        imports
                            .iter()
                            .map(|import| import.to_json_value(true))
                            .collect(),
                    ),
                )],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let imports = project.find_imports(query);
            success_response(
                &real_path,
                project.file_count(),
                [(
                    "imports",
                    Value::Array(
                        imports
                            .iter()
                            .map(|import| import.to_json_value(true))
                            .collect(),
                    ),
                )],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_annotations(path: &str, language: Option<&str>, query: &str) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_annotations(query) {
            Ok(annotations) => success_response(
                &real_path,
                db.file_count(),
                [(
                    "annotations",
                    Value::Array(
                        annotations
                            .iter()
                            .map(|annotation| annotation.to_json_value(true))
                            .collect(),
                    ),
                )],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let annotations = project.find_annotations(query);
            success_response(
                &real_path,
                project.file_count(),
                [(
                    "annotations",
                    Value::Array(
                        annotations
                            .iter()
                            .map(|annotation| annotation.to_json_value(true))
                            .collect(),
                    ),
                )],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_callers(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_callers(function_name, class_name) {
            Ok(callers) => success_response(
                &real_path,
                db.file_count(),
                [
                    ("function", json!(function_name)),
                    ("class_name", json!(class_name)),
                    ("callers", Value::Array(callers)),
                ],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let callers = project
                .find_callers(function_name, class_name)
                .into_iter()
                .map(|caller| caller.to_json_value())
                .collect::<Vec<_>>();
            success_response(
                &real_path,
                project.file_count(),
                [
                    ("function", json!(function_name)),
                    ("class_name", json!(class_name)),
                    ("callers", Value::Array(callers)),
                ],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_callees(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_callees(function_name, class_name) {
            Ok(callees) => success_response(
                &real_path,
                db.file_count(),
                [
                    ("function", json!(function_name)),
                    ("class_name", json!(class_name)),
                    ("callees", Value::Array(callees)),
                ],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let callees = project
                .find_callees(function_name, class_name)
                .into_iter()
                .map(|callee| callee.to_json_value())
                .collect::<Vec<_>>();
            success_response(
                &real_path,
                project.file_count(),
                [
                    ("function", json!(function_name)),
                    ("class_name", json!(class_name)),
                    ("callees", Value::Array(callees)),
                ],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_graph(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
    max_depth: usize,
    direction: GraphDirection,
) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => {
            match db.find_graphs(function_name, class_name, direction, max_depth) {
                Ok(graphs) => success_response(
                    &real_path,
                    db.file_count(),
                    [
                        ("function", json!(function_name)),
                        ("class_name", json!(class_name)),
                        ("direction", json!(direction.as_str())),
                        ("max_depth", json!(max_depth)),
                        ("graphs", json!(graphs)),
                    ],
                ),
                Err(error) => error_response(error),
            }
        }
        Ok(ProjectSource::Project(project)) => {
            match project.find_graphs(function_name, class_name, direction, max_depth) {
                Ok(graphs) => success_response(
                    &real_path,
                    project.file_count(),
                    [
                        ("function", json!(function_name)),
                        ("class_name", json!(class_name)),
                        ("direction", json!(direction.as_str())),
                        ("max_depth", json!(max_depth)),
                        ("graphs", json!(graphs)),
                    ],
                ),
                Err(error) => error_response(error),
            }
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_symbols(path: &str, language: Option<&str>, name: &str) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_symbols(name) {
            Ok(refs) if refs.is_empty() => success_response(
                &real_path,
                db.file_count(),
                [
                    ("name", json!(name)),
                    ("references", Value::Array(Vec::new())),
                ],
            ),
            Ok(refs) => match ProjectAnalyzer::new_with_language(&real_path, language) {
                Ok(project) => {
                    let refs = project
                        .hydrate_symbol_contexts(refs)
                        .into_iter()
                        .map(|symbol| symbol.to_json_value())
                        .collect::<Vec<_>>();
                    success_response(
                        &real_path,
                        db.file_count(),
                        [("name", json!(name)), ("references", Value::Array(refs))],
                    )
                }
                Err(error) => error_response(error),
            },
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let refs = project
                .find_symbols(name)
                .into_iter()
                .map(|symbol| symbol.to_json_value())
                .collect::<Vec<_>>();
            success_response(
                &real_path,
                project.file_count(),
                [("name", json!(name)), ("references", Value::Array(refs))],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_definition(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_functions(function_name) {
            Ok(functions) => {
                let functions = functions
                    .into_iter()
                    .filter(|function| {
                        function.name == function_name
                            && (class_name.is_none()
                                || function.class_name.as_deref() == class_name)
                    })
                    .collect::<Vec<_>>();
                if functions.is_empty() {
                    return json!({"error": format!("Function '{}' not found", function_name)});
                }
                match ProjectAnalyzer::new_with_language(&real_path, language) {
                    Ok(project) => {
                        let searched_files = unique_function_file_count(&functions);
                        let functions = project.hydrate_function_bodies(functions);
                        success_response(
                            &real_path,
                            searched_files,
                            [
                                ("class_name", json!(class_name)),
                                ("functions", functions_to_json(&functions, true)),
                            ],
                        )
                    }
                    Err(error) => error_response(error),
                }
            }
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let functions = project.find_function_definitions(function_name, class_name);
            if functions.is_empty() {
                return json!({"error": format!("Function '{}' not found", function_name)});
            }
            success_response(
                &real_path,
                project.file_count(),
                [
                    ("class_name", json!(class_name)),
                    ("functions", functions_to_json(&functions, true)),
                ],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_super_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_super_classes(class_name) {
            Ok(super_classes) => success_response(
                &real_path,
                db.file_count(),
                [
                    ("class_name", json!(class_name)),
                    (
                        "super_classes",
                        Value::Array(
                            super_classes
                                .iter()
                                .map(|class| class.to_json_value(true))
                                .collect(),
                        ),
                    ),
                ],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let super_classes = project.find_super_classes(class_name);
            success_response(
                &real_path,
                project.file_count(),
                [
                    ("class_name", json!(class_name)),
                    (
                        "super_classes",
                        Value::Array(
                            super_classes
                                .iter()
                                .map(|class| class.to_json_value(true))
                                .collect(),
                        ),
                    ),
                ],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_sub_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let real_path = crate::resolve_path(path);
    match with_project_source(&real_path, language) {
        Ok(ProjectSource::Database(db)) => match db.find_sub_classes(class_name) {
            Ok(sub_classes) => success_response(
                &real_path,
                db.file_count(),
                [
                    ("class_name", json!(class_name)),
                    (
                        "sub_classes",
                        Value::Array(
                            sub_classes
                                .iter()
                                .map(|class| class.to_json_value(true))
                                .collect(),
                        ),
                    ),
                ],
            ),
            Err(error) => error_response(error),
        },
        Ok(ProjectSource::Project(project)) => {
            let sub_classes = project.find_sub_classes(class_name);
            success_response(
                &real_path,
                project.file_count(),
                [
                    ("class_name", json!(class_name)),
                    (
                        "sub_classes",
                        Value::Array(
                            sub_classes
                                .iter()
                                .map(|class| class.to_json_value(true))
                                .collect(),
                        ),
                    ),
                ],
            )
        }
        Err(error) => error_response(error),
    }
}

pub(crate) fn cmd_index(path: &str, language: Option<&str>) -> Value {
    let real_path = crate::resolve_path(path);
    build_index(&real_path, language)
}

enum ProjectSource {
    Database(DbProjectAnalyzer),
    Project(ProjectAnalyzer),
}

fn with_project_source(path: &str, language: Option<&str>) -> anyhow::Result<ProjectSource> {
    if let Some(db) = DbProjectAnalyzer::from_current_dir_if_compatible(path, language)? {
        return Ok(ProjectSource::Database(db));
    }
    ProjectAnalyzer::new_with_language(path, language).map(ProjectSource::Project)
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

fn functions_to_json(functions: &[FunctionInfo], include_body: bool) -> Value {
    Value::Array(
        functions
            .iter()
            .map(|function| function.to_json_value(include_body, true))
            .collect(),
    )
}

fn unique_function_file_count(functions: &[FunctionInfo]) -> usize {
    functions
        .iter()
        .map(|function| function.location.file.as_str())
        .collect::<HashSet<_>>()
        .len()
}
