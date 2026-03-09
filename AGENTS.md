# AGENTS.md

## Build & Run
- Build: `cargo build` (debug) / `cargo build --release` (optimized, LTO enabled)
- Run: `cargo run -- <subcommand>` — binary name is `tsa`
- Lint: `cargo clippy`
- Format: `cargo fmt` — check with `cargo fmt -- --check`

## Architecture
Rust CLI tool using **clap** (derive) for arg parsing. Parses source code via **tree-sitter** grammars (Python, JS, TS, TSX, Java, Go) and provides structural analysis (functions, classes, imports, call graphs, inheritance, symbol references).
- `src/main.rs` — CLI entry point, subcommand dispatch, JSON output via `serde_json`
- `src/extractor.rs` — `CodeExtractor`: single-file extraction logic (functions, classes, calls, imports, refs)
- `src/parser/` — `BaseParser` + per-language modules (`python.rs`, `java.rs`, `go.rs`, `javascript.rs`) defining tree-sitter queries
- `src/db/` — SQLite index (`rusqlite`) for project-wide queries: schema, sync, query, prefilter, types, helpers
- `src/nodes.rs` — data structs (`FunctionInfo`, `ClassInfo`, etc.) for analysis results
- `src/source.rs` — `SourceAnalyzer`: multi-file analysis using `rayon` + `ignore` crate for gitignore-aware walking
- `src/graph.rs` — graph-based analysis utilities
- `src/index.rs` — `build_index`: indexes a project into SQLite
- `src/cache.rs` / `src/languages.rs` / `src/utils.rs` — internal helpers

## Code Style
- Rust 2021 edition. Error handling via `anyhow::Result`. Visibility: prefer `pub(crate)`.
- Imports: `std` first, then external crates, then `crate::` internal imports. Use `use crate::nodes::*` for node types.
- Naming: snake_case for functions/variables, PascalCase for types/enums. CLI enums derive `clap::ValueEnum`.
- Output is always JSON (via `serde_json::json!` macro). Print to stdout with `println!`.
