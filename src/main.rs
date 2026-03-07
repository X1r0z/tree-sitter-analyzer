mod analyzer;
mod cache;
mod db;
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

fn main() {
    process::exit(run());
}
