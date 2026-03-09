# AGENTS.md

- Build: `cargo build` (debug) / `cargo build --release` (optimized, LTO enabled)
- Run: `cargo run -- <subcommand>` — binary name is `tsa`
- Lint: `cargo clippy`
- Format: `cargo fmt` — check with `cargo fmt -- --check`

## Architecture
Rust CLI tool using **clap** (derive) for arg parsing. Parses source code via **tree-sitter** grammars (Python, JS, TS, TSX, Java, Go) and provides structural analysis (functions, classes, imports, call graphs, inheritance, symbol references).
Entry point: `src/main.rs`.
- `src/models.rs` – Core data types (`FunctionInfo`, `ClassInfo`, `Location`, etc.), all `Serialize`/`Deserialize`.
- `src/commands.rs` – Subcommand dispatch; uses `CommandContext` with `AnalyzerBackend` (source or indexed DB).
- `src/analyzers.rs` – `SourceAnalyzer` (live parse) and `StoreAnalyzer` (SQLite `tsa.db` index).
- `src/db/` – SQLite persistence via `rusqlite`; indexing, sync, and query.
- `src/parser/`, `src/query/` – Tree-sitter parsing and S-expression queries per language.
- `src/extractor.rs` – Extracts AST info from parsed trees.
- `src/graph.rs` – Multi-level call graph tracing.
- `src/languages.rs` – Language detection by extension.
- `src/search.rs` – Symbol reference search.
- `src/output.rs` – JSON output formatting.
- `src/traversal.rs` – File tree walking via `ignore` crate.
- `src/utils.rs` – Shared helpers: file discovery, progress bars, path relativization, result sorting.

## Code Style
- Rust 2021 edition. Use `anyhow::Result` for fallible functions.
- Imports: `std` first, blank line, external crates, blank line, `crate::` imports.
- Structs derive `Debug, Clone, Serialize, Deserialize`; use `#[serde(...)]` attributes for JSON field control.
- Parallelism via `rayon`; file walking via `ignore` crate (respects `.gitignore`).
- All output is JSON to stdout; errors/progress to stderr. No `println!` for data—use `serde_json`.
