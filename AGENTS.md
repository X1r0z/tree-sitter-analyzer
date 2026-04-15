# AGENTS.md

- Build: `cargo build` (debug) / `cargo build --release` (optimized, LTO enabled)
- Run: `cargo run -- <subcommand>` — binary name is `tsa`
- Test: `cargo test` (all) / `cargo test <test_name>` (single test, e.g. `cargo test test_functions`)
- Lint: `cargo clippy`
- Format: `cargo fmt` — check with `cargo fmt -- --check`

## Design Philosophy
This tool primarily assists LLM Agents in code auditing. SAST analysis intentionally allows over-approximations (false positives) to avoid missing real issues—prefer recall over precision.

## Architecture
Rust CLI tool using **clap** (derive) for arg parsing. Parses source code via **tree-sitter** grammars (Python, JS, TS, TSX, Java, Go) to build a SQLite index, then answers structural queries (functions, classes, imports, call graphs, inheritance, symbol references) from that index.
Entry point: `src/main.rs`.
- `src/models.rs` – Core data types (`FunctionInfo`, `ClassInfo`, `Location`, `IndexInfo`, etc.), all shared across extraction, analysis, traversal, and JSON output.
- `src/commands.rs` – Subcommand dispatch and command execution flow; ensures `tsa.db` exists and is compatible before running queries.
- `src/analyzer.rs` – SQLite-backed query backend used by all query commands.
- `src/extractor.rs` – Extracts AST information from parsed trees.
- `src/languages.rs` – Language detection by extension and supported-language metadata.
- `src/output.rs` – JSON output formatting for stdout.
- `src/traversal.rs` – Shared traversal utilities for graph and hierarchy analysis.
- `src/utils.rs` – Shared helpers: file discovery, progress bars, path relativization, and result sorting.
- `src/parser/` – Tree-sitter parsing support and language-specific parsing helpers.
- `src/query/` – Indexed query layer for lookups, call edges, call graphs, and class hierarchy queries.
- `src/db/` – SQLite persistence, index snapshots, synchronization, and store/query support.

## Code Style
- Rust 2021 edition. Use `anyhow::Result` for fallible functions.
- Imports: `std` first, blank line, external crates, blank line, `crate::` imports.
- Structs derive `Debug, Clone, Serialize, Deserialize`; use `#[serde(...)]` attributes for JSON field control.
- Parallelism via `rayon`; file walking via `ignore` crate (respects `.gitignore`).
- All output is JSON to stdout; errors/progress to stderr. No `println!` for data—use `serde_json`.
- Do not write unnecessary wrapper functions. If a function is only called from one place, prefer inlining it unless the extraction materially improves readability or reuse.
