mod analyzer;
mod languages;
mod nodes;
mod output;
mod parallel;
mod parser;
mod project;
mod utils;

use std::path::Path;
use std::process;

use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use crate::output::print_pretty;
use crate::project::ProjectAnalyzer;

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
    /// Extract all function/method definitions
    Functions {
        /// Directory path
        path: String,
        /// Filter by function name (regex match)
        #[arg(short, long)]
        query: Option<String>,
        /// Include function body
        #[arg(long)]
        body: bool,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Extract all class/struct/interface definitions
    Classes {
        /// Directory path
        path: String,
        /// Filter by class name (regex match)
        #[arg(short, long)]
        query: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Get all fields of a specific class
    Fields {
        /// Directory path
        path: String,
        /// Class name to get fields for
        #[arg(short, long)]
        class_name: String,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Extract all import statements
    Imports {
        /// Directory path
        path: String,
        /// Filter by module name (regex match)
        #[arg(short, long)]
        query: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Extract all variable declarations
    Variables {
        /// Directory path
        path: String,
        /// Filter by variable name (regex match)
        #[arg(short, long)]
        query: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Find functions that call a specific function
    Callers {
        /// Directory path
        path: String,
        /// Function name to find callers for
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Find functions called by a specific function
    Callees {
        /// Directory path
        path: String,
        /// Function name to find callees for
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Find all references to a specific identifier
    Symbols {
        /// Directory path
        path: String,
        /// Identifier name to search for
        #[arg(short, long)]
        name: String,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Get the complete source code of a function
    Definition {
        /// Directory path
        path: String,
        /// Function name to retrieve
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Get all variables declared in a function
    #[command(name = "function-variables")]
    FunctionVariables {
        /// Directory path
        path: String,
        /// Function name to analyze
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Get all string literals in a function
    #[command(name = "function-strings")]
    FunctionStrings {
        /// Directory path
        path: String,
        /// Function name to analyze
        #[arg(short, long)]
        function: String,
        /// Class name to filter methods
        #[arg(short, long)]
        class_name: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Get all parent classes of a specific class
    #[command(name = "super-classes")]
    SuperClasses {
        /// Directory path
        path: String,
        /// Class name to find parents for
        #[arg(short, long)]
        class_name: String,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
    },
    /// Get all child classes that inherit from a class
    #[command(name = "sub-classes")]
    SubClasses {
        /// Directory path
        path: String,
        /// Class name to find children for
        #[arg(short, long)]
        class_name: String,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// Output in YAML format
        #[arg(long)]
        yaml: bool,
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

fn output_result(result: &Value, use_json: bool, use_yaml: bool) {
    if use_json {
        println!(
            "{}",
            serde_json::to_string_pretty(result).unwrap_or_default()
        );
    } else if use_yaml {
        println!("{}", serde_yaml::to_string(result).unwrap_or_default());
    } else {
        print_pretty(result);
    }
}

fn run() -> i32 {
    let cli = Cli::parse();

    let (result, use_json, use_yaml) = match cli.command {
        Commands::Functions {
            path,
            query,
            body,
            json,
            yaml,
        } => {
            let result = cmd_functions(&path, query.as_deref().unwrap_or(""), body);
            (result, json, yaml)
        }
        Commands::Classes {
            path,
            query,
            json,
            yaml,
        } => {
            let result = cmd_classes(&path, query.as_deref().unwrap_or(""));
            (result, json, yaml)
        }
        Commands::Fields {
            path,
            class_name,
            json,
            yaml,
        } => {
            let result = cmd_fields(&path, &class_name);
            (result, json, yaml)
        }
        Commands::Imports {
            path,
            query,
            json,
            yaml,
        } => {
            let result = cmd_imports(&path, query.as_deref().unwrap_or(""));
            (result, json, yaml)
        }
        Commands::Variables {
            path,
            query,
            json,
            yaml,
        } => {
            let result = cmd_variables(&path, query.as_deref().unwrap_or(""));
            (result, json, yaml)
        }
        Commands::Callers {
            path,
            function,
            class_name,
            json,
            yaml,
        } => {
            let result = cmd_callers(&path, &function, class_name.as_deref());
            (result, json, yaml)
        }
        Commands::Callees {
            path,
            function,
            class_name,
            json,
            yaml,
        } => {
            let result = cmd_callees(&path, &function, class_name.as_deref());
            (result, json, yaml)
        }
        Commands::Symbols {
            path,
            name,
            json,
            yaml,
        } => {
            let result = cmd_symbols(&path, &name);
            (result, json, yaml)
        }
        Commands::Definition {
            path,
            function,
            class_name,
            json,
            yaml,
        } => {
            let result = cmd_definition(&path, &function, class_name.as_deref());
            (result, json, yaml)
        }
        Commands::FunctionVariables {
            path,
            function,
            class_name,
            json,
            yaml,
        } => {
            let result = cmd_function_variables(&path, &function, class_name.as_deref());
            (result, json, yaml)
        }
        Commands::FunctionStrings {
            path,
            function,
            class_name,
            json,
            yaml,
        } => {
            let result = cmd_function_strings(&path, &function, class_name.as_deref());
            (result, json, yaml)
        }
        Commands::SuperClasses {
            path,
            class_name,
            json,
            yaml,
        } => {
            let result = cmd_super_classes(&path, &class_name);
            (result, json, yaml)
        }
        Commands::SubClasses {
            path,
            class_name,
            json,
            yaml,
        } => {
            let result = cmd_sub_classes(&path, &class_name);
            (result, json, yaml)
        }
    };

    output_result(&result, use_json, use_yaml);
    if result.get("error").is_some() {
        1
    } else {
        0
    }
}

fn cmd_functions(path: &str, query: &str, include_body: bool) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let functions = project.get_functions(query);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": functions.len(),
                "functions": functions.iter().map(|f| f.to_json_value(include_body, true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_classes(path: &str, query: &str) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let classes = project.get_classes(query);
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

fn cmd_fields(path: &str, class_name: &str) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let fields = project.get_fields(class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "class_name": class_name,
                "count": fields.len(),
                "fields": fields.iter().map(|f| f.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_imports(path: &str, query: &str) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let imports = project.get_imports(query);
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

fn cmd_variables(path: &str, query: &str) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let variables = project.get_variables(query);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "count": variables.len(),
                "variables": variables.iter().map(|v| v.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_callers(path: &str, function_name: &str, class_name: Option<&str>) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let callers = project.get_callers(function_name, class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "function": function_name,
                "class_name": class_name,
                "count": callers.len(),
                "callers": callers,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_callees(path: &str, function_name: &str, class_name: Option<&str>) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let callees = project.get_callees(function_name, class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "function": function_name,
                "class_name": class_name,
                "count": callees.len(),
                "callees": callees,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_symbols(path: &str, name: &str) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let refs = project.find_symbols(name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "name": name,
                "count": refs.len(),
                "references": refs,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_definition(path: &str, function_name: &str, class_name: Option<&str>) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let functions = project.get_all_functions_by_name(function_name, class_name);
            if functions.is_empty() {
                return json!({"error": format!("Function '{}' not found", function_name)});
            }
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "class_name": class_name,
                "count": functions.len(),
                "functions": functions.iter().map(|f| f.to_json_value(true, true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_function_variables(path: &str, function_name: &str, class_name: Option<&str>) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let functions = project.get_all_functions_by_name(function_name, class_name);
            if functions.is_empty() {
                return json!({"error": format!("Function '{}' not found", function_name)});
            }
            let variables = project.get_function_variables(function_name, class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "function": function_name,
                "class_name": class_name,
                "count": variables.len(),
                "variables": variables,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_function_strings(path: &str, function_name: &str, class_name: Option<&str>) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let functions = project.get_all_functions_by_name(function_name, class_name);
            if functions.is_empty() {
                return json!({"error": format!("Function '{}' not found", function_name)});
            }
            let strings = project.get_function_strings(function_name, class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "function": function_name,
                "class_name": class_name,
                "count": strings.len(),
                "strings": strings,
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_super_classes(path: &str, class_name: &str) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let super_classes = project.get_super_classes(class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "class_name": class_name,
                "count": super_classes.len(),
                "super_classes": super_classes.iter().map(|c| c.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_sub_classes(path: &str, class_name: &str) -> Value {
    let real_path = resolve_path(path);
    match ProjectAnalyzer::new(&real_path) {
        Ok(project) => {
            let sub_classes = project.get_sub_classes(class_name);
            json!({
                "path": real_path,
                "files_searched": project.files.len(),
                "class_name": class_name,
                "count": sub_classes.len(),
                "sub_classes": sub_classes.iter().map(|c| c.to_json_value(true)).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn main() {
    process::exit(run());
}
