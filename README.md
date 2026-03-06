# tree-sitter-analyzer

A code analysis toolkit using [tree-sitter](https://tree-sitter.github.io/tree-sitter/) for AST-based code structure extraction, call graph analysis, and symbol reference tracking.

## Features

- **Code Structure Extraction** - Extract all function/method and class/struct/interface/field definitions
- **Inheritance Analysis** - Extract class inheritance relationships, including parent classes and child classes
- **Call Graph Analysis** - Build call graphs showing caller-callee relationships
- **Import Analysis** - Extract import statements and dependencies
- **Symbol Reference Tracking** - Find all references to a specific symbol
- **Project Indexing** - Build a persistent `tsa.db` cache for faster repeated project queries

## Supported Languages

| Language | Extensions |
|----------|------------|
| Python | `.py`, `.pyw`, `.pyi` |
| JavaScript | `.js`, `.mjs`, `.cjs`, `.jsx` |
| TypeScript | `.ts`, `.tsx` |
| Java | `.java` |
| Go | `.go` |

Use `-l, --language <LANGUAGE>` to restrict AST parsing to one language. Allowed values: `python`, `java`, `go`, `javascript`, `typescript`, `tsx`.

## Installation

```bash
cargo install --git https://github.com/X1r0z/tree-sitter-analyzer
```

## Usage

```bash
# List all classes in a directory
tsa classes ./src/

# Build an index for all supported languages in the current project
tsa index .

# Build an index for Java files only
tsa index . -l java

# Only analyze Python files
tsa functions ./src/ -l python

# Find all callers of a function
tsa callers ./src/ --function process_data

# Get function definition
tsa definition ./src/ --function main
```

See [USAGE.md](USAGE.md) for complete documentation.

## Tools

| Tool | Description |
|------|-------------|
| `functions` | Extract all function/method definitions |
| `classes` | Extract all class/struct/interface definitions |
| `fields` | Extract all field definitions |
| `imports` | Extract all import statements |
| `callers` | Find functions that call a specific function |
| `callees` | Find functions called by a specific function |
| `symbols` | Find all references to a specific symbol |
| `definition` | Extract source code of a function |
| `super-classes` | Get all parent classes of a specific class |
| `sub-classes` | Get all child classes that inherit from a specific class |
| `index` | Build a persistent project index in `./tsa.db` |

## Development

```bash
# Build
cargo build --release

# Lint
cargo clippy

# Format
cargo fmt
```

## License

MIT
