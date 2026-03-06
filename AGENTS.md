# AGENTS.md

## Commands
- Build: `cargo build --release`
- Run: `cargo run --release -- <command> <path> [options]` (binary name: `tsa`)
- Lint: `cargo clippy`
- Format: `cargo fmt`
- Test: `cargo test`

## Architecture
- **src/main.rs** - CLI entry point using clap derive API, command dispatch, output formatting
- **src/parser.rs** - BaseParser: core tree-sitter AST parsing, query execution, extraction of functions/classes/calls/imports/fields
- **src/analyzer.rs** - CodeAnalyzer: high-level single-file analysis facade wrapping BaseParser + AnalyzerCache
- **src/cache.rs** - AnalyzerCache: lazy memoization of parsed results behind Mutex
- **src/project.rs** - ProjectAnalyzer: multi-file parallel analysis using Rayon
- **src/languages.rs** - Language configs: parser factories, tree-sitter queries, extension mapping
- **src/nodes.rs** - Data structures (Location, FunctionInfo, ClassInfo, CallInfo, ImportInfo, FieldInfo, etc.)
- **src/utils.rs** - File discovery (using `ignore` crate) and regex-based symbol filtering
- Supported languages: Python, JavaScript/TypeScript, Java, Go

## Code Style
- Rust 2021 edition; use `anyhow` for error handling
- Use `rayon` for parallel file analysis, `serde` + `serde_json` for JSON output
- Use `streaming-iterator` for tree-sitter query cursor iteration
- Visibility: prefer `pub(crate)` for internal APIs; only `pub` for items used outside the crate
- Naming: snake_case for functions/variables, PascalCase for types/structs, UPPER_SNAKE for constants
