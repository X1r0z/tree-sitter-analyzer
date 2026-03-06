# Tree-sitter Analyzer

Command-line interface for analyzing code using tree-sitter AST parsing.

## Installation

```bash
cargo install --git https://github.com/X1r0z/tree-sitter-analyzer
```

After installation, the `tsa` command will be available.

## Basic Usage

```bash
tsa <command> <path> [options]
```

### Supported Languages

| Language | Extensions |
|----------|------------|
| Python | `.py`, `.pyw`, `.pyi` |
| JavaScript | `.js`, `.mjs`, `.cjs`, `.jsx` |
| TypeScript | `.ts`, `.tsx` |
| Java | `.java` |
| Go | `.go` |

### Output Formats

By default, output is in human-readable format. Use `--json` or `--yaml` flag for structured output.

| Option | Description |
|--------|-------------|
| `--json` | Output in JSON format |
| `--yaml` | Output in YAML format |

```bash
# Human-readable output
tsa functions ./src/

# JSON output
tsa functions ./src/ --json

# YAML output
tsa functions ./src/ --yaml
```

## Commands

### Code Structure

#### `functions` - Extract Function Definitions

Extract all function/method definitions from source code.

```bash
tsa functions <path> [-q QUERY] [--body] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by function name (regex match) |
| `--body` | Include function body in output |

Examples:

```bash
# List all functions in a directory
tsa functions ./src/

# Filter functions containing "get"
tsa functions ./src/ -q get

# Filter functions starting with "get_" using regex
tsa functions ./src/ -q "^get_"

# Include function bodies
tsa functions ./src/ --body

# Output as JSON
tsa functions ./src/ --json

# Output as YAML
tsa functions ./src/ --yaml
```

#### `classes` - Extract Class Definitions

Extract all class/struct/interface definitions.

```bash
tsa classes <path> [-q QUERY] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by class name (regex match) |

Examples:

```bash
# List all classes in a directory
tsa classes ./src/

# Filter classes containing "Handler"
tsa classes ./src/ -q Handler

# Filter classes ending with "Service" using regex
tsa classes ./src/ -q "Service$"
```

#### `fields` - Get Class Fields

Get all fields of a specific class.

```bash
tsa fields <path> -c CLASS_NAME [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name to get fields for (required) |

Examples:

```bash
# Search across a project
tsa fields ./src/ -c DatabaseConfig
```

#### `imports` - Extract Import Statements

Extract all import statements from source code.

```bash
tsa imports <path> [-q QUERY] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by module name (regex match) |

Examples:

```bash
# List all imports in a directory
tsa imports ./src/

# Find imports containing "json"
tsa imports ./src/ -q json

# Find imports matching "^(os|sys)$" using regex
tsa imports ./src/ -q "^(os|sys)$"
```

### Inheritance Analysis

#### `super-classes` - Get Parent Classes

Get all parent classes (superclasses) of a specific class.

```bash
tsa super-classes <path> -c CLASS_NAME [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name to find parents for (required) |

Examples:

```bash
# Find parent classes
tsa super-classes ./src/ -c AdminUser
```

#### `sub-classes` - Get Child Classes

Get all child classes (subclasses) that inherit from a specific class.

```bash
tsa sub-classes <path> -c CLASS_NAME [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name to find children for (required) |

Examples:

```bash
# Find child classes across a project
tsa sub-classes ./src/ -c BaseModel
```

### Call Graph Analysis

#### `callers` - Find Function Callers

Find all functions that call a specific function.

```bash
tsa callers <path> -f FUNCTION [-c CLASS_NAME] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name to find callers for (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Find all callers of a function
tsa callers ./src/ -f process_data

# Find callers of a method within a class
tsa callers ./src/ -f save -c DatabaseHandler
```

#### `callees` - Find Called Functions

Find all functions called by a specific function.

```bash
tsa callees <path> -f FUNCTION [-c CLASS_NAME] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name to find callees for (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Find all functions called by main
tsa callees ./src/ -f main

# Find callees of a method
tsa callees ./src/ -f initialize -c Application
```

### Function-Level Analysis

#### `definition` - Get Function Source Code

Get the complete source code of a specific function.

```bash
tsa definition <path> -f FUNCTION [-c CLASS_NAME] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name to retrieve (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Get function definition
tsa definition ./src/ -f parse_config

# Get method definition from a class
tsa definition ./src/ -f connect -c Database
```

### Symbol Reference Tracking

#### `symbols` - Find Symbol References

Find all references to a specific identifier.

```bash
tsa symbols <path> -n NAME [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-n, --name` | Identifier name to search for (required) |

Examples:

```bash
# Find all references to a symbol
tsa symbols ./src/ -n CONFIG_PATH
```
