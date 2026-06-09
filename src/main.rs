mod analyzer;
mod commands;
mod db;
mod extractor;
mod hydrator;
mod indexer;
mod languages;
mod models;
mod output;
mod parser;
mod query;
mod traversal;
mod utils;

use std::process;

use clap::{Parser, Subcommand, ValueEnum};

use crate::models::GraphDirection;
use crate::utils::relativize_paths;

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
    /// Build a persistent project index cache
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
    Refs {
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

fn parse_positive_depth(value: &str) -> Result<usize, String> {
    let depth: usize = value
        .parse()
        .map_err(|_| format!("invalid depth '{value}'"))?;
    if depth == 0 {
        return Err("depth must be >= 1".to_string());
    }
    Ok(depth)
}

#[allow(clippy::too_many_lines)]
fn dispatch(command: Commands) -> serde_json::Value {
    match command {
        Commands::Index { path, language } => {
            commands::index(&path, language.map(LanguageFilter::as_str))
        }
        Commands::Functions {
            path,
            language,
            query,
        } => commands::functions(
            &path,
            language.map(LanguageFilter::as_str),
            query.as_deref().unwrap_or(""),
        ),
        Commands::Classes {
            path,
            language,
            query,
        } => commands::classes(
            &path,
            language.map(LanguageFilter::as_str),
            query.as_deref().unwrap_or(""),
        ),
        Commands::Fields {
            path,
            language,
            class_name,
        } => commands::fields(&path, language.map(LanguageFilter::as_str), &class_name),
        Commands::Imports {
            path,
            language,
            query,
        } => commands::imports(
            &path,
            language.map(LanguageFilter::as_str),
            query.as_deref().unwrap_or(""),
        ),
        Commands::Annotations {
            path,
            language,
            query,
        } => commands::annotations(
            &path,
            language.map(LanguageFilter::as_str),
            query.as_deref().unwrap_or(""),
        ),
        Commands::Callers {
            path,
            language,
            function,
            class_name,
        } => commands::callers(
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
        } => commands::callees(
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
        } => commands::graph(
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
        Commands::Refs {
            path,
            language,
            name,
        } => commands::refs(&path, language.map(LanguageFilter::as_str), &name),
        Commands::Definition {
            path,
            language,
            function,
            class_name,
        } => commands::definition(
            &path,
            language.map(LanguageFilter::as_str),
            &function,
            class_name.as_deref(),
        ),
        Commands::SuperClasses {
            path,
            language,
            class_name,
        } => commands::super_classes(&path, language.map(LanguageFilter::as_str), &class_name),
        Commands::SubClasses {
            path,
            language,
            class_name,
        } => commands::sub_classes(&path, language.map(LanguageFilter::as_str), &class_name),
    }
}

fn run() -> i32 {
    let cli = Cli::parse();
    let mut result = dispatch(cli.command);

    if result.get("error").is_none() {
        let root_path = result["meta"]["root"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        relativize_paths(&mut result, &root_path);
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    i32::from(result.get("error").is_some())
}

fn main() {
    process::exit(run());
}
