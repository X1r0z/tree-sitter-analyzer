# tree-sitter-analyzer

A code analysis toolkit using [tree-sitter](https://tree-sitter.github.io/tree-sitter/) for AST-based code structure extraction, call graph analysis, and symbol reference tracking.

## Features

- **Function/Class Extraction** - Extract all function/method and class/struct/interface/field definitions
- **Inheritance Analysis** - Extract class inheritance relationships, including parent classes and child classes
- **Call Graph Analysis** - Build call graphs showing caller-callee relationships
- **Import Analysis** - Extract import statements and dependencies
- **Symbol Reference Tracking** - Find all references to a specific symbol

## Supported Languages

| Language | Extensions |
|----------|------------|
| Python | `.py`, `.pyw`, `.pyi` |
| JavaScript | `.js`, `.mjs`, `.cjs`, `.jsx` |
| TypeScript | `.ts`, `.tsx` |
| Java | `.java` |
| Go | `.go` |

## Installation

```bash
cargo install --git https://github.com/X1r0z/tree-sitter-analyzer
```

## Usage

```bash
# List all classes in a directory
tsa classes ./src/

# Find all callers of a function
tsa callers ./src/ --function process_data

# Get function definition
tsa definition ./src/ --function main

# Output as JSON
tsa functions ./src/ --json

# Output as YAML
tsa functions ./src/ --yaml
```

See [USAGE.md](USAGE.md) for complete documentation.

## Modules

### Code Structure

| Tool | Description |
|------|-------------|
| `functions` | Extract all function/method definitions |
| `classes` | Extract all class/struct/interface definitions |
| `fields` | Extract all field definitions |
| `imports` | Extract all import statements |
| `definition` | Get the complete source code of a function |

### Inheritance Analysis

| Tool | Description |
|------|-------------|
| `super-classes` | Get all parent classes of a specific class |
| `sub-classes` | Get all child classes that inherit from a specific class |

### Call Graph Analysis

| Tool | Description |
|------|-------------|
| `callers` | Find functions that call a specific function |
| `callees` | Find functions called by a specific function |

### Symbol Reference Tracking

| Tool | Description |
|------|-------------|
| `symbols` | Find all references to a specific symbol |

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
