# tree-sitter-analyzer

A code analysis toolkit using [tree-sitter](https://tree-sitter.github.io/tree-sitter/) for AST-based code structure extraction, call graph analysis, and symbol reference tracking.

## Features

- **Code Structure Extraction** - Extract all function/method and class/struct/interface/field definitions
- **Inheritance Analysis** - Extract class inheritance relationships, including parent classes and child classes
- **Call Graph Analysis** - Build call graphs showing caller-callee relationships
- **Import Analysis** - Extract import statements and dependencies
- **Annotation Analysis** - Extract Java annotations and Python decorators with their targets
- **Symbol Reference Tracking** - Find identifier, type, property, field, and import-path references for a specific code symbol
- **Project Indexing** - Build a persistent cache for faster repeated project queries

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

# Trace callers backward in the call graph
tsa graph ./src/ --function process_data --depth 2 --backward

# Trace callees forward in the call graph
tsa graph ./src/ --function process_data --depth 2 --forward

# Get function definition
tsa definition ./src/ --function main
```

See [USAGE.md](USAGE.md) for complete documentation.

## Tools

| Tool | Description |
|------|-------------|
| `index` | Build a persistent project index |
| `functions` | Extract all function/method definitions |
| `classes` | Extract all class/struct/interface definitions |
| `fields` | Extract all field definitions |
| `imports` | Extract all import statements |
| `annotations` | Extract Java annotations and Python decorators |
| `callers` | Find functions that call a specific function |
| `callees` | Find functions called by a specific function |
| `graph` | Trace callers backward or callees forward across multiple call levels |
| `refs` | Find all references to a specific symbol |
| `definition` | Extract source code of a function |
| `super-classes` | Get all parent classes of a specific class |
| `sub-classes` | Get all child classes that inherit from a specific class |

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
