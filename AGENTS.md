# AGENTS.md

- Build: `cargo build` (debug) / `cargo build --release` (optimized, LTO enabled)
- Run: `cargo run -- <subcommand>` — binary name is `tsa`
- Test: `cargo test` (all) / `cargo test <test_name>` (single test, e.g. `cargo test test_functions`)
- Lint: `cargo clippy`
- Format: `cargo fmt` — check with `cargo fmt -- --check`

## Design Philosophy
This tool primarily assists LLM Agents in code auditing. SAST analysis intentionally allows over-approximations (false positives) to avoid missing real issues—prefer recall over precision.

## Architecture
Rust CLI tool using **clap** (derive) for arg parsing. Parses source code via **tree-sitter** grammars (Python, JS, TS, TSX, Java, Go) to build a SQLite index (`tsa.db`), then answers structural queries (functions, classes, imports, call graphs, inheritance, symbol references) from that index.

Flow: a query command resolves its path, calls `indexer::ensure_index` (incrementally builds/refreshes `tsa.db`), opens a `CodeAnalyzer` over the index, runs the query through the `query/` layer, then hydrates and formats results. The index stores lightweight metadata; full function bodies and reference contexts are re-parsed on demand by the hydrator.

Entry point: `src/main.rs` – CLI definition (clap), subcommand dispatch, and path relativization of the final JSON.
- `src/commands.rs` – Per-subcommand execution flow; wires together indexing, analysis, hydration, and output into JSON responses.
- `src/indexer.rs` – Builds and incrementally refreshes the SQLite index; walks files, parses in parallel (`rayon`), and ensures the index covers the requested path/language.
- `src/analyzer.rs` – SQLite-backed query backend; thin handle that delegates to the `query/` layer used by all query commands.
- `src/extractor.rs` – Extracts AST information (functions, classes, calls, imports, refs, etc.) from a parsed file, with per-file caching.
- `src/hydrator.rs` – Lazily fills in full function bodies and reference contexts for query results by re-parsing the relevant files.
- `src/models.rs` – Core data types (`FunctionInfo`, `ClassInfo`, `Location`, `IndexInfo`, key types, etc.) shared across extraction, analysis, traversal, and JSON output.
- `src/languages.rs` – Language detection by extension, supported-language metadata, and compiled tree-sitter queries.
- `src/traversal.rs` – Shared graph traversal (DFS path collection) for call-graph and hierarchy analysis.
- `src/output.rs` – JSON output formatting for stdout.
- `src/utils.rs` – Shared helpers: file discovery (via `ignore`), progress bars, path resolution/relativization, and response envelopes.
- `src/parser/` – Tree-sitter parsing core (`ParseContext`, capture matching, call-target resolution) plus per-language extraction logic under `languages/`.
- `src/query/` – Indexed query layer over `tsa.db`: lookups, call edges (callers/callees), call graphs, class hierarchy, call resolution, and regex prefiltering.
- `src/db/` – SQLite persistence: schema/migrations, the `IndexStore`, snapshot writing, and incremental synchronization.

## Code Style
- Rust 2021 edition. Use `anyhow::Result` for fallible functions.
- Imports: `std` first, blank line, external crates, blank line, `crate::` imports.
- Structs derive `Debug, Clone, Serialize, Deserialize`; use `#[serde(...)]` attributes for JSON field control.
- Parallelism via `rayon`; file walking via `ignore` crate (respects `.gitignore`).
- All output is JSON to stdout; errors/progress to stderr. No `println!` for data—use `serde_json`.
- Do not write unnecessary wrapper functions. If a function is only called from one place, prefer inlining it unless the extraction materially improves readability or reuse.
