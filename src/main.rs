mod analyzer;
mod cache;
mod commands;
mod db;
mod graph;
mod index;
mod languages;
mod models;
mod parser;
mod project;
mod utils;

use std::path::Path;
use std::process;

use clap::{Parser, Subcommand, ValueEnum};

use crate::commands::{
    cmd_annotations, cmd_callees, cmd_callers, cmd_classes, cmd_definition, cmd_fields,
    cmd_functions, cmd_graph, cmd_imports, cmd_index, cmd_sub_classes, cmd_super_classes,
    cmd_symbols,
};
use crate::models::GraphDirection;
use crate::utils::relativize_json_file_paths;

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

pub(crate) fn resolve_path(path: &str) -> String {
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
    let mut result = match cli.command {
        Commands::Functions {
            path,
            language,
            query,
        } => cmd_functions(
            &path,
            language.map(LanguageFilter::as_str),
            query.as_deref().unwrap_or(""),
        ),
        Commands::Classes {
            path,
            language,
            query,
        } => cmd_classes(
            &path,
            language.map(LanguageFilter::as_str),
            query.as_deref().unwrap_or(""),
        ),
        Commands::Fields {
            path,
            language,
            class_name,
        } => cmd_fields(&path, language.map(LanguageFilter::as_str), &class_name),
        Commands::Imports {
            path,
            language,
            query,
        } => cmd_imports(
            &path,
            language.map(LanguageFilter::as_str),
            query.as_deref().unwrap_or(""),
        ),
        Commands::Callers {
            path,
            language,
            function,
            class_name,
        } => cmd_callers(
            &path,
            language.map(LanguageFilter::as_str),
            &function,
            class_name.as_deref(),
        ),
        Commands::Callees {
            path,
            language,
            function,
            class_name,
        } => cmd_callees(
            &path,
            language.map(LanguageFilter::as_str),
            &function,
            class_name.as_deref(),
        ),
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
            language.map(LanguageFilter::as_str),
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
        } => cmd_symbols(&path, language.map(LanguageFilter::as_str), &name),
        Commands::Definition {
            path,
            language,
            function,
            class_name,
        } => cmd_definition(
            &path,
            language.map(LanguageFilter::as_str),
            &function,
            class_name.as_deref(),
        ),
        Commands::SuperClasses {
            path,
            language,
            class_name,
        } => cmd_super_classes(&path, language.map(LanguageFilter::as_str), &class_name),
        Commands::SubClasses {
            path,
            language,
            class_name,
        } => cmd_sub_classes(&path, language.map(LanguageFilter::as_str), &class_name),
        Commands::Annotations {
            path,
            language,
            query,
        } => cmd_annotations(
            &path,
            language.map(LanguageFilter::as_str),
            query.as_deref().unwrap_or(""),
        ),
        Commands::Index { path, language } => {
            cmd_index(&path, language.map(LanguageFilter::as_str))
        }
    };

    if result.get("error").is_none() {
        let root_path = result["path"].as_str().unwrap_or_default().to_string();
        relativize_json_file_paths(&mut result, &root_path);
    }

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

fn main() {
    process::exit(run());
}
