# AGENTS.md

## Commands
- Build: `cargo build --release`
- Run: `./target/release/tree-sitter-analyzer <command> <path> [options]`
- Lint: `cargo clippy`
- Format: `cargo fmt`
- Test: `cargo test`

## Architecture
- **src/main.rs** - CLI entry point using clap, command dispatch, output formatting
- **src/analyzer.rs** - CodeAnalyzer: single-file AST parsing, call graphs, inheritance
- **src/project.rs** - ProjectAnalyzer: multi-file parallel analysis using Rayon
- **src/languages.rs** - Language configs: parsers, queries, extension mapping
- **src/nodes.rs** - Data structures (Location, FunctionInfo, ClassInfo, etc.)
- **src/output.rs** - Human-readable output formatting
- Supported languages: Python, JavaScript/TypeScript, Java, Go

## Code Style
- Rust 2021 edition
- Use `anyhow` for error handling
- Use `rayon` for data-parallel file analysis
- Use `serde` + `serde_json` for JSON output, `serde_yaml` for YAML
- Use `clap` derive API for CLI argument parsing
- Use `streaming-iterator` for tree-sitter query iteration
