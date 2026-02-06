# Tree-sitter Analyzer CLI

Command-line interface for analyzing code using tree-sitter AST parsing.

## Installation

```bash
# Using uv (recommended)
uv tool install git+https://github.com/X1r0z/tree-sitter-mcp

# Using pip
pip install git+https://github.com/X1r0z/tree-sitter-mcp
```

After installation, the `tree-sitter-analyzer` command will be available.

## Basic Usage

```bash
tree-sitter-analyzer <command> <path> [options]
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
tree-sitter-analyzer functions ./src/

# JSON output
tree-sitter-analyzer functions ./src/ --json

# YAML output
tree-sitter-analyzer functions ./src/ --yaml
```

## Commands

### Code Structure

#### `functions` - Extract Function Definitions

Extract all function/method definitions from source code.

```bash
tree-sitter-analyzer functions <path> [-q QUERY] [--body] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by function name (regex match) |
| `--body` | Include function body in output |

Examples:

```bash
# List all functions in a directory
tree-sitter-analyzer functions ./src/

# Filter functions containing "get"
tree-sitter-analyzer functions ./src/ -q get

# Filter functions starting with "get_" using regex
tree-sitter-analyzer functions ./src/ -q "^get_"

# Include function bodies
tree-sitter-analyzer functions ./src/ --body

# Output as JSON
tree-sitter-analyzer functions ./src/ --json

# Output as YAML
tree-sitter-analyzer functions ./src/ --yaml
```

#### `classes` - Extract Class Definitions

Extract all class/struct/interface definitions.

```bash
tree-sitter-analyzer classes <path> [-q QUERY] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by class name (regex match) |

Examples:

```bash
# List all classes in a directory
tree-sitter-analyzer classes ./src/

# Filter classes containing "Handler"
tree-sitter-analyzer classes ./src/ -q Handler

# Filter classes ending with "Service" using regex
tree-sitter-analyzer classes ./src/ -q "Service$"
```

#### `fields` - Get Class Fields

Get all fields of a specific class.

```bash
tree-sitter-analyzer fields <path> -c CLASS_NAME [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name to get fields for (required) |

Examples:

```bash
# Search across a project
tree-sitter-analyzer fields ./src/ -c DatabaseConfig
```

#### `imports` - Extract Import Statements

Extract all import statements from source code.

```bash
tree-sitter-analyzer imports <path> [-q QUERY] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by module name (regex match) |

Examples:

```bash
# List all imports in a directory
tree-sitter-analyzer imports ./src/

# Find imports containing "json"
tree-sitter-analyzer imports ./src/ -q json

# Find imports matching "^(os|sys)$" using regex
tree-sitter-analyzer imports ./src/ -q "^(os|sys)$"
```

#### `variables` - Extract Variable Declarations

Extract all variable declarations with scope information.

```bash
tree-sitter-analyzer variables <path> [-q QUERY] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by variable name (regex match) |

Examples:

```bash
# List all variables in a directory
tree-sitter-analyzer variables ./src/

# Find variables containing "config"
tree-sitter-analyzer variables ./src/ -q config

# Find variables matching uppercase pattern using regex
tree-sitter-analyzer variables ./src/ -q "^[A-Z_]+$"
```

### Inheritance Analysis

#### `super-classes` - Get Parent Classes

Get all parent classes (superclasses) of a specific class.

```bash
tree-sitter-analyzer super-classes <path> -c CLASS_NAME [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name to find parents for (required) |

Examples:

```bash
# Find parent classes
tree-sitter-analyzer super-classes ./src/ -c AdminUser
```

#### `sub-classes` - Get Child Classes

Get all child classes (subclasses) that inherit from a specific class.

```bash
tree-sitter-analyzer sub-classes <path> -c CLASS_NAME [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name to find children for (required) |

Examples:

```bash
# Find child classes across a project
tree-sitter-analyzer sub-classes ./src/ -c BaseModel
```

### Call Graph Analysis

#### `callers` - Find Function Callers

Find all functions that call a specific function.

```bash
tree-sitter-analyzer callers <path> -f FUNCTION [-c CLASS_NAME] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name to find callers for (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Find all callers of a function
tree-sitter-analyzer callers ./src/ -f process_data

# Find callers of a method within a class
tree-sitter-analyzer callers ./src/ -f save -c DatabaseHandler
```

#### `callees` - Find Called Functions

Find all functions called by a specific function.

```bash
tree-sitter-analyzer callees <path> -f FUNCTION [-c CLASS_NAME] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name to find callees for (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Find all functions called by main
tree-sitter-analyzer callees ./src/ -f main

# Find callees of a method
tree-sitter-analyzer callees ./src/ -f initialize -c Application
```

### Function-Level Analysis

#### `definition` - Get Function Source Code

Get the complete source code of a specific function.

```bash
tree-sitter-analyzer definition <path> -f FUNCTION [-c CLASS_NAME] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name to retrieve (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Get function definition
tree-sitter-analyzer definition ./src/ -f parse_config

# Get method definition from a class
tree-sitter-analyzer definition ./src/ -f connect -c Database
```

#### `function-variables` - Get Variables in Function

Get all variables declared within a specific function.

```bash
tree-sitter-analyzer function-variables <path> -f FUNCTION [-c CLASS_NAME] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name to analyze (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Get variables in a function
tree-sitter-analyzer function-variables ./src/ -f process_request
```

#### `function-strings` - Get Strings in Function

Get all string literals within a specific function.

```bash
tree-sitter-analyzer function-strings <path> -f FUNCTION [-c CLASS_NAME] [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name to analyze (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Get string literals in a function
tree-sitter-analyzer function-strings ./src/ -f handle_request
```

### Symbol Reference Tracking

#### `symbols` - Find Symbol References

Find all references to a specific identifier.

```bash
tree-sitter-analyzer symbols <path> -n NAME [--json] [--yaml]
```

| Option | Description |
|--------|-------------|
| `-n, --name` | Identifier name to search for (required) |

Examples:

```bash
# Find all references to a symbol
tree-sitter-analyzer symbols ./src/ -n CONFIG_PATH
```
