---
name: "tree-sitter-analyzer"
description: "AST code analysis via tree-sitter (structure, calls, inheritance, symbols). Invoke when you need structural code insights from files or directories."
allowed-tools: Bash(tree-sitter-analyzer:*)
---

# Tree-sitter Analyzer

Tree-sitter Analyzer is a code analysis toolkit built on tree-sitter AST parsing.

Supported languages: Python, JavaScript/TypeScript, Java, Go.

## When to Use This Skill

Use this skill whenever you need fast, structural answers from a codebase without relying on regex-only searches.

Prefer this skill over Grep for code understanding tasks, because it uses the parsed AST (not plain text) and can answer questions like “who calls this?”, “what does this function call?”, and “what fields does this class define?” reliably across formatting differences.

Only fall back to Grep when you truly need an exact text match (e.g., a specific string literal), or when the target language/file type is not supported by this skill.

Examples of structural questions this skill answers well:

- Enumerate functions/methods/classes and their locations across a repository
- Inspect class fields and high-level class structure (methods/fields lists)
- Understand inheritance relationships (super-classes / sub-classes)
- Build a lightweight call graph view (who calls X, what X calls)
- Track symbol references (identifier occurrences with context and locations)
- Extract variables or string literals inside a specific function for auditing/debugging

This skill is especially useful for:

- Onboarding into an unfamiliar codebase
- Refactoring and impact analysis (e.g., “who calls this?”)
- Security reviews and audits (e.g., “where is this token/string used?”)
- Generating an architectural inventory of a project (APIs, modules, entry points)

## CLI Tool Overview

After installation, the `tree-sitter-analyzer` command is available.

### Installation

```bash
# Using uv (recommended)
uv tool install git+https://github.com/X1r0z/tree-sitter-mcp

# Using pip
pip install git+https://github.com/X1r0z/tree-sitter-mcp
```

### Basic Usage

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

By default, output is in human-readable format. Use `--json` flag for structured output.

| Option | Description |
|--------|-------------|
| `--json` | Output in JSON format |

```bash
# Human-readable output
tree-sitter-analyzer functions ./src/

# JSON output
tree-sitter-analyzer functions ./src/ --json
```

## Commands

### Code Structure

#### `functions` - Extract Function Definitions

Extract all function/method definitions from source code.

```bash
tree-sitter-analyzer functions <path> [-q QUERY] [--body] [--json]
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
```

#### `classes` - Extract Class Definitions

Extract all class/struct/interface definitions.

```bash
tree-sitter-analyzer classes <path> [-q QUERY] [--json]
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
tree-sitter-analyzer fields <path> -c CLASS_NAME [--json]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name to get fields for (required) |

Examples:

```bash
# Get fields of a class
tree-sitter-analyzer fields ./src/models.py -c User

# Search across a project
tree-sitter-analyzer fields ./src/ -c DatabaseConfig
```

#### `imports` - Extract Import Statements

Extract all import statements from source code.

```bash
tree-sitter-analyzer imports <path> [-q QUERY] [--json]
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
tree-sitter-analyzer variables <path> [-q QUERY] [--json]
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
tree-sitter-analyzer super-classes <path> -c CLASS_NAME [--json]
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
tree-sitter-analyzer sub-classes <path> -c CLASS_NAME [--json]
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
tree-sitter-analyzer callers <path> -f FUNCTION [-c CLASS_NAME] [--json]
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
tree-sitter-analyzer callees <path> -f FUNCTION [-c CLASS_NAME] [--json]
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
tree-sitter-analyzer definition <path> -f FUNCTION [-c CLASS_NAME] [--json]
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
tree-sitter-analyzer function-variables <path> -f FUNCTION [-c CLASS_NAME] [--json]
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
tree-sitter-analyzer function-strings <path> -f FUNCTION [-c CLASS_NAME] [--json]
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
tree-sitter-analyzer symbols <path> -n NAME [--json]
```

| Option | Description |
|--------|-------------|
| `-n, --name` | Identifier name to search for (required) |

Examples:

```bash
# Find all references to a symbol
tree-sitter-analyzer symbols ./src/ -n CONFIG_PATH
```

## Typical Workflows

### Replace common Grep use-cases with AST queries

```bash
# "Where is this function defined?"
tree-sitter-analyzer definition /path/to/project -f <FUNCTION> --json

# "Who calls this function?"
tree-sitter-analyzer callers /path/to/project -f <FUNCTION> --json

# "What does this function call?"
tree-sitter-analyzer callees /path/to/project -f <FUNCTION> --json

# "Where is this identifier used?"
tree-sitter-analyzer symbols /path/to/project -n <NAME> --json

# "Show me all classes / all functions" (optionally filter by fuzzy query)
tree-sitter-analyzer classes /path/to/project -q <QUERY>
tree-sitter-analyzer functions /path/to/project -q <QUERY>
```

### Inventory a repository (functions/classes/imports)

```bash
tree-sitter-analyzer functions /path/to/project --json
tree-sitter-analyzer classes /path/to/project --json
tree-sitter-analyzer imports /path/to/project --json
```

### Find impact of a change (callers/callees)

```bash
tree-sitter-analyzer callers /path/to/project -f process_data --json
tree-sitter-analyzer callees /path/to/project -f process_data --json
```

### Track a symbol through a project

```bash
tree-sitter-analyzer symbols /path/to/project -n CONFIG_PATH --json
```

### Inspect a specific function in detail

```bash
tree-sitter-analyzer definition /path/to/project -f main --json
tree-sitter-analyzer function-variables /path/to/project -f main --json
tree-sitter-analyzer function-strings /path/to/project -f main --json
```

## Example Output

The following examples show the structured JSON produced by the CLI (`--json`). The JSON shape matches the actual CLI outputs.

### `functions`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "count": 2,
  "functions": [
    {
      "name": "main",
      "start_line": 1,
      "end_line": 24,
      "file": "/path/to/project/src/app.py"
    },
    {
      "name": "connect",
      "start_line": 10,
      "end_line": 55,
      "file": "/path/to/project/src/db.py",
      "is_method": true,
      "class_name": "Database"
    }
  ]
}
```

### `classes`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "count": 1,
  "classes": [
    {
      "name": "Database",
      "start_line": 1,
      "end_line": 120,
      "methods": [
        "__init__",
        "connect",
        "close"
      ],
      "fields": [
        "dsn",
        "timeout"
      ],
      "file": "/path/to/project/src/db.py"
    }
  ]
}
```

### `fields`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "class_name": "Database",
  "count": 2,
  "fields": [
    {
      "name": "dsn",
      "line": 5,
      "file": "/path/to/project/src/db.py",
      "type": "str",
      "class_name": "Database"
    },
    {
      "name": "timeout",
      "line": 6,
      "file": "/path/to/project/src/db.py",
      "type": "int",
      "class_name": "Database"
    }
  ]
}
```

### `imports`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "count": 2,
  "imports": [
    {
      "module": "os",
      "line": 1,
      "file": "/path/to/project/src/app.py"
    },
    {
      "module": "json",
      "line": 2,
      "file": "/path/to/project/src/app.py"
    }
  ]
}
```

### `variables`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "count": 2,
  "variables": [
    {
      "name": "CONFIG_PATH",
      "line": 3,
      "scope": null,
      "file": "/path/to/project/src/config.py"
    },
    {
      "name": "payload",
      "line": 18,
      "scope": "main",
      "file": "/path/to/project/src/app.py"
    }
  ]
}
```

### `super-classes`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "class_name": "AdminUser",
  "count": 1,
  "super_classes": [
    {
      "name": "User",
      "start_line": 1,
      "end_line": 80,
      "methods": [
        "__init__",
        "save"
      ],
      "fields": [
        "id",
        "email"
      ],
      "file": "/path/to/project/src/models.py"
    }
  ]
}
```

### `sub-classes`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "class_name": "User",
  "count": 1,
  "sub_classes": [
    {
      "name": "AdminUser",
      "start_line": 82,
      "end_line": 140,
      "methods": [
        "has_permission"
      ],
      "fields": [
        "role"
      ],
      "file": "/path/to/project/src/models.py"
    }
  ]
}
```

### `callers`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "function": "process_data",
  "class_name": null,
  "count": 2,
  "callers": [
    {
      "caller": "main",
      "line": 12,
      "file": "/path/to/project/src/app.py",
      "target_class": null
    },
    {
      "caller": "handle_request",
      "line": 88,
      "file": "/path/to/project/src/api.py",
      "target_class": null
    }
  ]
}
```

### `callees`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "function": "main",
  "class_name": null,
  "count": 2,
  "callees": [
    {
      "callee": "load_config",
      "line": 18,
      "file": "/path/to/project/src/app.py",
      "class_name": null
    },
    {
      "callee": "process_data",
      "line": 25,
      "file": "/path/to/project/src/app.py",
      "class_name": null
    }
  ]
}
```

### `definition`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "class_name": null,
  "count": 1,
  "functions": [
    {
      "name": "main",
      "start_line": 10,
      "end_line": 30,
      "file": "/path/to/project/src/app.py",
      "body": "def main():\\n    config = load_config(CONFIG_PATH)\\n    return process_data(config)\\n"
    }
  ]
}
```

### `function-variables`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "function": "main",
  "class_name": null,
  "count": 2,
  "variables": [
    {
      "name": "config",
      "line": 11,
      "file": "/path/to/project/src/app.py"
    },
    {
      "name": "result",
      "line": 12,
      "file": "/path/to/project/src/app.py"
    }
  ]
}
```

### `function-strings`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "function": "main",
  "class_name": null,
  "count": 1,
  "strings": [
    {
      "value": "starting...",
      "line": 15,
      "file": "/path/to/project/src/app.py"
    }
  ]
}
```

### `symbols`

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "name": "CONFIG_PATH",
  "count": 2,
  "references": [
    {
      "type": "identifier",
      "location": {
        "file": "/path/to/project/src/config.py",
        "start_line": 3,
        "end_line": 3
      },
      "context": "CONFIG_PATH = \"/etc/myapp/config.json\""
    },
    {
      "type": "identifier",
      "location": {
        "file": "/path/to/project/src/app.py",
        "start_line": 18,
        "end_line": 18
      },
      "context": "load_config(CONFIG_PATH)"
    }
  ]
}
```
