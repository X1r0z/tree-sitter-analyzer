mod analyzer;
mod cache;
mod db;
mod graph;
mod index;
mod languages;
mod nodes;
mod parser;
mod project;
mod utils;

use std::collections::HashSet;
use std::path::Path;
use std::process;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};

use crate::db::DbProjectAnalyzer;
use crate::index::build_index;
use crate::nodes::GraphDirection;
use crate::project::ProjectAnalyzer;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LanguageFilter {
    Python,
    Java,
    Go,
    #[value(name = "javascript")]
    JavaScript,
    #[value(name = "typescript")]
    TypeScript,
    Tsx,
}

impl LanguageFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Java => "java",
            Self::Go => "go",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
        }
    }
}

#[derive(Parser)]
#[command(
    name = "tree-sitter-analyzer",
    version = "0.1.0",
    about = "Tree-sitter based code analyzer for extracting code structure and relationships",
    after_help = r#"Supported languages: Python, JavaScript/TypeScript, Java, Go"#
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a persistent project index in ./tsa.db
    Index {
        /// Directory path
        path: String,
        /// Only index files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
    },
    /// Extract all function/method definitions
    Functions {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Filter by function name (regex match)
        #[arg(short, long)]
        query: Option<String>,
    },
    /// Extract all class/struct/interface definitions
    Classes {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Filter by class name (regex match)
        #[arg(short, long)]
        query: Option<String>,
    },
    /// Get all fields of a specific class
    Fields {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Class name to get fields for
        #[arg(short, long)]
        class_name: String,
    },
    /// Extract all import statements
    Imports {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Filter by module name (regex match)
        #[arg(short, long)]
        query: Option<String>,
    },
    /// Extract annotations (Java) / decorators (Python)
    Annotations {
        /// Directory path
        path: String,
        /// Only analyze files for a single language (java or python)
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Filter by annotation/decorator name (regex match)
        #[arg(short, long)]
        query: Option<String>,
    },
    /// Find functions that call a specific function
    Callers {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Function name to find callers for
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
    },
    /// Find functions called by a specific function
    Callees {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Function name to find callees for
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
    },
    /// Trace function or method call graphs
    Graph {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Function name to trace
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
        /// Maximum call depth to trace
        #[arg(short, long, value_parser = parse_positive_depth)]
        depth: usize,
        /// Trace callers backward
        #[arg(long, conflicts_with = "forward", required_unless_present = "forward")]
        backward: bool,
        /// Trace callees forward
        #[arg(
            long,
            conflicts_with = "backward",
            required_unless_present = "backward"
        )]
        forward: bool,
    },
    /// Find all references to a specific identifier
    Symbols {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Identifier name to search for
        #[arg(short, long)]
        name: String,
    },
    /// Get the complete source code of a function
    Definition {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Function name to retrieve
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
    },
    /// Get all parent classes of a specific class
    #[command(name = "super-classes")]
    SuperClasses {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Class name to find parents for
        #[arg(short, long)]
        class_name: String,
    },
    /// Get all child classes that inherit from a class
    #[command(name = "sub-classes")]
    SubClasses {
        /// Directory path
        path: String,
        /// Only analyze files for a single language
        #[arg(short = 'l', long, value_enum)]
        language: Option<LanguageFilter>,
        /// Class name to find children for
        #[arg(short, long)]
        class_name: String,
    },
}

fn resolve_path(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => {
            let p = Path::new(path);
            if p.is_absolute() {
                path.to_string()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path).to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.to_string())
            }
        }
    }
}

fn parse_positive_depth(value: &str) -> Result<usize, String> {
    let depth: usize = value
        .parse()
        .map_err(|_| format!("invalid depth '{}'", value))?;
    if depth == 0 {
        return Err("depth must be >= 1".to_string());
    }
    Ok(depth)
}

fn run() -> i32 {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Functions {
            path,
            language,
            query,
        } => cmd_functions(&path, language, query.as_deref().unwrap_or("")),
        Commands::Classes {
            path,
            language,
            query,
        } => cmd_classes(&path, language, query.as_deref().unwrap_or("")),
        Commands::Fields {
            path,
            language,
            class_name,
        } => cmd_fields(&path, language, &class_name),
        Commands::Imports {
            path,
            language,
            query,
        } => cmd_imports(&path, language, query.as_deref().unwrap_or("")),
        Commands::Callers {
            path,
            language,
            function,
            class_name,
        } => cmd_callers(&path, language, &function, class_name.as_deref()),
        Commands::Callees {
            path,
            language,
            function,
            class_name,
        } => cmd_callees(&path, language, &function, class_name.as_deref()),
        Commands::Graph {
            path,
            language,
            function,
            class_name,
            depth,
            backward,
            forward: _,
        } => cmd_graph(
            &path,
            language,
            &function,
            class_name.as_deref(),
            depth,
            if backward {
                GraphDirection::Backward
            } else {
                GraphDirection::Forward
            },
        ),
        Commands::Symbols {
            path,
            language,
            name,
        } => cmd_symbols(&path, language, &name),
        Commands::Definition {
            path,
            language,
            function,
            class_name,
        } => cmd_definition(&path, language, &function, class_name.as_deref()),
        Commands::SuperClasses {
            path,
            language,
            class_name,
        } => cmd_super_classes(&path, language, &class_name),
        Commands::SubClasses {
            path,
            language,
            class_name,
        } => cmd_sub_classes(&path, language, &class_name),
        Commands::Annotations {
            path,
            language,
            query,
        } => cmd_annotations(&path, language, query.as_deref().unwrap_or("")),
        Commands::Index { path, language } => cmd_index(&path, language),
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    if result.get("error").is_some() {
        1
    } else {
        0
    }
}

fn cmd_functions(path: &str, language: Option<LanguageFilter>, query: &str) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_functions(query) {
            Ok(functions) => {
                return match ProjectAnalyzer::new_with_language(
                    &real_path,
                    language.map(LanguageFilter::as_str),
                ) {
                    Ok(project) => {
                        let functions = project.hydrate_function_bodies(functions);
                        json!({
                            "path": real_path,
                            "files_searched": db.file_count(),
                            "count": functions.len(),
                            "functions": functions.iter().map(|f| f.to_json_value(false, true)).collect::<Vec<_>>(),
                        })
                    }
                    Err(e) => json!({"error": e.to_string()}),
                };
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let functions = project.find_functions(query);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": functions.len(),
                "functions": functions.iter().map(|f| f.to_json_value(false, true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_classes(path: &str, language: Option<LanguageFilter>, query: &str) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_classes(query) {
            Ok(classes) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": classes.len(),
                    "classes": classes.iter().map(|c| c.to_json_value(true)).collect::<Vec<_>>(),
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let classes = project.find_classes(query);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": classes.len(),
                "classes": classes.iter().map(|c| c.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_fields(path: &str, language: Option<LanguageFilter>, class_name: &str) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_fields(class_name) {
            Ok(fields) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": fields.len(),
                    "class_name": class_name,
                    "fields": fields.iter().map(|f| f.to_json_value(true)).collect::<Vec<_>>(),
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let fields = project.find_fields(class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": fields.len(),
                "class_name": class_name,
                "fields": fields.iter().map(|f| f.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_imports(path: &str, language: Option<LanguageFilter>, query: &str) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_imports(query) {
            Ok(imports) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": imports.len(),
                    "imports": imports.iter().map(|i| i.to_json_value(true)).collect::<Vec<_>>(),
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let imports = project.find_imports(query);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": imports.len(),
                "imports": imports.iter().map(|i| i.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_callers(
    path: &str,
    language: Option<LanguageFilter>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_callers(function_name, class_name) {
            Ok(callers) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": callers.len(),
                    "function": function_name,
                    "class_name": class_name,
                    "callers": callers,
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let callers = project.find_callers(function_name, class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": callers.len(),
                "function": function_name,
                "class_name": class_name,
                "callers": callers,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_callees(
    path: &str,
    language: Option<LanguageFilter>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_callees(function_name, class_name) {
            Ok(callees) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": callees.len(),
                    "function": function_name,
                    "class_name": class_name,
                    "callees": callees,
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let callees = project.find_callees(function_name, class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": callees.len(),
                "function": function_name,
                "class_name": class_name,
                "callees": callees,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_graph(
    path: &str,
    language: Option<LanguageFilter>,
    function_name: &str,
    class_name: Option<&str>,
    max_depth: usize,
    direction: GraphDirection,
) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_graphs(function_name, class_name, direction, max_depth) {
            Ok(graphs) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": graphs.len(),
                    "function": function_name,
                    "class_name": class_name,
                    "direction": direction.as_str(),
                    "max_depth": max_depth,
                    "graphs": graphs,
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }

    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => match project.find_graphs(function_name, class_name, direction, max_depth) {
            Ok(graphs) => json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": graphs.len(),
                "function": function_name,
                "class_name": class_name,
                "direction": direction.as_str(),
                "max_depth": max_depth,
                "graphs": graphs,
            }),
            Err(e) => json!({"error": e.to_string()}),
        },
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_symbols(path: &str, language: Option<LanguageFilter>, name: &str) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_symbols(name) {
            Ok(refs) => {
                if refs.is_empty() {
                    return json!({
                        "path": real_path,
                        "files_searched": db.file_count(),
                        "count": 0,
                        "name": name,
                        "references": Vec::<Value>::new(),
                    });
                }
                return match ProjectAnalyzer::new_with_language(
                    &real_path,
                    language.map(LanguageFilter::as_str),
                ) {
                    Ok(project) => {
                        let refs = project.hydrate_symbol_contexts(refs);
                        json!({
                            "path": real_path,
                            "files_searched": db.file_count(),
                            "count": refs.len(),
                            "name": name,
                            "references": refs.iter().map(|symbol| symbol.to_json_value()).collect::<Vec<_>>(),
                        })
                    }
                    Err(e) => json!({"error": e.to_string()}),
                };
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let refs = project.find_symbols(name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": refs.len(),
                "name": name,
                "references": refs,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_definition(
    path: &str,
    language: Option<LanguageFilter>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_functions(function_name) {
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
                return match ProjectAnalyzer::new_with_language(
                    &real_path,
                    language.map(LanguageFilter::as_str),
                ) {
                    Ok(project) => {
                        let files_searched = unique_function_file_count(&functions);
                        let functions = project.hydrate_function_bodies(functions);
                        json!({
                            "path": real_path,
                            "files_searched": files_searched,
                            "count": functions.len(),
                            "class_name": class_name,
                            "functions": functions.iter().map(|f| f.to_json_value(true, true)).collect::<Vec<_>>(),
                        })
                    }
                    Err(e) => json!({"error": e.to_string()}),
                };
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }

    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let functions = project.find_function_definitions(function_name, class_name);
            if functions.is_empty() {
                return json!({"error": format!("Function '{}' not found", function_name)});
            }
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": functions.len(),
                "class_name": class_name,
                "functions": functions.iter().map(|f| f.to_json_value(true, true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_super_classes(path: &str, language: Option<LanguageFilter>, class_name: &str) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_super_classes(class_name) {
            Ok(super_classes) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": super_classes.len(),
                    "class_name": class_name,
                    "super_classes": super_classes.iter().map(|c| c.to_json_value(true)).collect::<Vec<_>>(),
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let super_classes = project.find_super_classes(class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": super_classes.len(),
                "class_name": class_name,
                "super_classes": super_classes.iter().map(|c| c.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_sub_classes(path: &str, language: Option<LanguageFilter>, class_name: &str) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_sub_classes(class_name) {
            Ok(sub_classes) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": sub_classes.len(),
                    "class_name": class_name,
                    "sub_classes": sub_classes.iter().map(|c| c.to_json_value(true)).collect::<Vec<_>>(),
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let sub_classes = project.find_sub_classes(class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": sub_classes.len(),
                "class_name": class_name,
                "sub_classes": sub_classes.iter().map(|c| c.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_annotations(path: &str, language: Option<LanguageFilter>, query: &str) -> Value {
    let real_path = resolve_path(path);
    if let Ok(Some(db)) = DbProjectAnalyzer::from_current_dir_if_compatible(
        &real_path,
        language.map(LanguageFilter::as_str),
    ) {
        match db.find_annotations(query) {
            Ok(annotations) => {
                return json!({
                    "path": real_path,
                    "files_searched": db.file_count(),
                    "count": annotations.len(),
                    "annotations": annotations.iter().map(|a| a.to_json_value(true)).collect::<Vec<_>>(),
                });
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    match ProjectAnalyzer::new_with_language(&real_path, language.map(LanguageFilter::as_str)) {
        Ok(project) => {
            let annotations = project.find_annotations(query);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": annotations.len(),
                "annotations": annotations.iter().map(|a| a.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_index(path: &str, language: Option<LanguageFilter>) -> Value {
    let real_path = resolve_path(path);
    build_index(&real_path, language.map(LanguageFilter::as_str))
}

fn unique_function_file_count(functions: &[crate::nodes::FunctionInfo]) -> usize {
    functions
        .iter()
        .map(|function| function.location.file.as_str())
        .collect::<HashSet<_>>()
        .len()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    use clap::Parser;
    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;

    fn cwd_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn write_test_project(root: &Path) {
        fs::write(
            root.join("app.py"),
            r#"def leaf():
    print("leaf")

def shared():
    leaf()

def start():
    shared()

class App:
    @property
    def value(self):
        return leaf()

    def read(self):
        return self.value

    def loop(self):
        return helper()

    def helper(self):
        return loop()
"#,
        )
        .unwrap();

        fs::write(
            root.join("other.py"),
            r#"def leaf():
    return 1

def shared():
    leaf()

def caller():
    shared()
"#,
        )
        .unwrap();
    }

    fn write_java_test_project(root: &Path) {
        fs::write(
            root.join("Controller.java"),
            r#"public class Controller {
    private final Service service = new Service();

    public String list() {
        return service.list();
    }
}
"#,
        )
        .unwrap();

        fs::write(
            root.join("Service.java"),
            r#"public class Service {
    public String list() {
        return helper();
    }

    public String helper() {
        return "ok";
    }
}
"#,
        )
        .unwrap();
    }

    fn with_test_project<R>(f: impl FnOnce(&TempDir) -> R) -> R {
        let _guard = cwd_lock().lock().unwrap_or_else(|error| error.into_inner());
        let tempdir = TempDir::new().unwrap();
        write_test_project(tempdir.path());
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(tempdir.path()).unwrap();
        let result = f(&tempdir);
        std::env::set_current_dir(previous).unwrap();
        result
    }

    fn with_java_test_project<R>(f: impl FnOnce(&TempDir) -> R) -> R {
        let _guard = cwd_lock().lock().unwrap_or_else(|error| error.into_inner());
        let tempdir = TempDir::new().unwrap();
        write_java_test_project(tempdir.path());
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(tempdir.path()).unwrap();
        let result = f(&tempdir);
        std::env::set_current_dir(previous).unwrap();
        result
    }

    fn graph_names(value: &Value) -> Vec<Vec<String>> {
        value["graphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|graph| {
                graph["stacktrace"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item.as_str().unwrap().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn graph_requires_direction_and_positive_depth() {
        assert!(Cli::try_parse_from(["tsa", "graph", ".", "-f", "start", "-d", "2"]).is_err());
        assert!(Cli::try_parse_from([
            "tsa",
            "graph",
            ".",
            "-f",
            "start",
            "-d",
            "2",
            "--backward",
            "--forward"
        ])
        .is_err());
        assert!(
            Cli::try_parse_from(["tsa", "graph", ".", "-f", "start", "-d", "0", "--backward"])
                .is_err()
        );
    }

    #[test]
    fn graph_forward_without_index_returns_project_only_paths() {
        with_test_project(|tempdir| {
            let value = cmd_graph(
                tempdir.path().to_str().unwrap(),
                Some(LanguageFilter::Python),
                "start",
                None,
                2,
                GraphDirection::Forward,
            );

            assert_eq!(value["count"].as_u64(), Some(1));
            let stacktraces = graph_names(&value);
            assert_eq!(
                stacktraces,
                vec![vec![
                    "start".to_string(),
                    "shared".to_string(),
                    "leaf".to_string(),
                ]]
            );
            for graph in value["graphs"].as_array().unwrap() {
                assert_eq!(graph["depth"].as_u64(), Some(2));
                assert_eq!(
                    graph["stacktrace"].as_array().unwrap().len(),
                    graph["path"].as_array().unwrap().len()
                );
            }
        });
    }

    #[test]
    fn graph_includes_all_matching_start_nodes() {
        with_test_project(|tempdir| {
            let value = cmd_graph(
                tempdir.path().to_str().unwrap(),
                Some(LanguageFilter::Python),
                "shared",
                None,
                1,
                GraphDirection::Forward,
            );

            assert_eq!(value["count"].as_u64(), Some(2));
            assert_eq!(
                graph_names(&value),
                vec![
                    vec!["shared".to_string(), "leaf".to_string()],
                    vec!["shared".to_string(), "leaf".to_string()],
                ]
            );
        });
    }

    #[test]
    fn graph_backward_supports_python_property_callers() {
        with_test_project(|tempdir| {
            let value = cmd_graph(
                tempdir.path().to_str().unwrap(),
                Some(LanguageFilter::Python),
                "value",
                Some("App"),
                1,
                GraphDirection::Backward,
            );

            assert_eq!(value["count"].as_u64(), Some(1));
            assert_eq!(
                graph_names(&value),
                vec![vec!["App.value".to_string(), "App.read".to_string()]]
            );
            let path = &value["graphs"][0]["path"];
            assert_eq!(path[0]["class_name"].as_str(), Some("App"));
            assert_eq!(path[1]["class_name"].as_str(), Some("App"));
        });
    }

    #[test]
    fn graph_stops_at_cycles_and_filters_builtins() {
        with_test_project(|tempdir| {
            let leaf = cmd_graph(
                tempdir.path().to_str().unwrap(),
                Some(LanguageFilter::Python),
                "leaf",
                None,
                1,
                GraphDirection::Forward,
            );
            assert_eq!(leaf["count"].as_u64(), Some(2));
            assert!(leaf["graphs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|graph| graph["depth"].as_u64() == Some(0)));

            let cycle = cmd_graph(
                tempdir.path().to_str().unwrap(),
                Some(LanguageFilter::Python),
                "loop",
                Some("App"),
                3,
                GraphDirection::Forward,
            );
            assert_eq!(
                graph_names(&cycle),
                vec![vec!["App.loop".to_string(), "App.helper".to_string()]]
            );
        });
    }

    #[test]
    fn graph_matches_with_and_without_index() {
        with_test_project(|tempdir| {
            let root = tempdir.path().to_str().unwrap();
            let without_index = cmd_graph(
                root,
                Some(LanguageFilter::Python),
                "start",
                None,
                2,
                GraphDirection::Forward,
            );

            let index_result = cmd_index(".", Some(LanguageFilter::Python));
            assert!(index_result.get("error").is_none(), "{index_result}");

            let with_index = cmd_graph(
                root,
                Some(LanguageFilter::Python),
                "start",
                None,
                2,
                GraphDirection::Forward,
            );

            assert_eq!(without_index["graphs"], with_index["graphs"]);
        });
    }

    #[test]
    fn graph_resolves_java_field_method_calls() {
        with_java_test_project(|tempdir| {
            let forward = cmd_graph(
                tempdir.path().to_str().unwrap(),
                Some(LanguageFilter::Java),
                "list",
                Some("Controller"),
                2,
                GraphDirection::Forward,
            );
            assert_eq!(
                graph_names(&forward),
                vec![vec![
                    "Controller.list".to_string(),
                    "Service.list".to_string(),
                    "Service.helper".to_string(),
                ]]
            );

            let backward = cmd_graph(
                tempdir.path().to_str().unwrap(),
                Some(LanguageFilter::Java),
                "list",
                Some("Service"),
                1,
                GraphDirection::Backward,
            );
            assert_eq!(
                graph_names(&backward),
                vec![vec![
                    "Service.list".to_string(),
                    "Controller.list".to_string()
                ]]
            );
        });
    }
}

fn main() {
    process::exit(run());
}
