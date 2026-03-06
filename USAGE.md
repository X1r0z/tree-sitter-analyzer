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

Global per-command filter:

- `-l, --language <LANGUAGE>`: only parse files for one language
- Allowed values: `python`, `java`, `go`, `javascript`, `typescript`, `tsx`

### Supported Languages

| Language | Extensions |
|----------|------------|
| Python | `.py`, `.pyw`, `.pyi` |
| JavaScript | `.js`, `.mjs`, `.cjs`, `.jsx` |
| TypeScript | `.ts`, `.tsx` |
| Java | `.java` |
| Go | `.go` |

## Commands

### Code Structure

#### `functions` - Extract Function Definitions

Extract all function/method definitions from source code.

```bash
tsa functions <path> [-l LANGUAGE] [-q QUERY] [--body]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-q, --query` | Filter by function name (regex match) |
| `--body` | Include function body in output |

Examples:

```bash
# List all functions in a directory
tsa functions ./src/

# Only analyze TypeScript files
tsa functions ./src/ -l typescript

# Filter functions containing "get"
tsa functions ./src/ -q get

# Filter functions starting with "get_" using regex
tsa functions ./src/ -q "^get_"

# Include function bodies
tsa functions ./src/ --body
```

#### `classes` - Extract Class Definitions

Extract all class/struct/interface definitions.

```bash
tsa classes <path> [-l LANGUAGE] [-q QUERY]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
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
tsa fields <path> [-l LANGUAGE] -c CLASS_NAME
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-c, --class-name` | Class name to get fields for (required) |

Examples:

```bash
# Search across a project
tsa fields ./src/ -c DatabaseConfig
```

#### `imports` - Extract Import Statements

Extract all import statements from source code.

```bash
tsa imports <path> [-l LANGUAGE] [-q QUERY]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
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

#### `definition` - Get Function Source Code

Get the complete source code of a specific function.

```bash
tsa definition <path> [-l LANGUAGE] -f FUNCTION [-c CLASS_NAME]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-f, --function` | Function name to retrieve (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Get function definition
tsa definition ./src/ -f parse_config

# Get method definition from a class
tsa definition ./src/ -f connect -c Database
```

### Inheritance Analysis

#### `super-classes` - Get Parent Classes

Get all parent classes (superclasses) of a specific class.

```bash
tsa super-classes <path> [-l LANGUAGE] -c CLASS_NAME
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-c, --class-name` | Class name to find parents for (required) |

Examples:

```bash
# Find parent classes
tsa super-classes ./src/ -c AdminUser
```

#### `sub-classes` - Get Child Classes

Get all child classes (subclasses) that inherit from a specific class.

```bash
tsa sub-classes <path> [-l LANGUAGE] -c CLASS_NAME
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
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
tsa callers <path> [-l LANGUAGE] -f FUNCTION [-c CLASS_NAME]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
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
tsa callees <path> [-l LANGUAGE] -f FUNCTION [-c CLASS_NAME]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-f, --function` | Function name to find callees for (required) |
| `-c, --class-name` | Class name to filter methods |

Examples:

```bash
# Find all functions called by main
tsa callees ./src/ -f main

# Find callees of a method
tsa callees ./src/ -f initialize -c Application
```

### Symbol Reference Tracking

#### `symbols` - Find Symbol References

Find all references to a specific identifier.

```bash
tsa symbols <path> [-l LANGUAGE] -n NAME
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-n, --name` | Identifier name to search for (required) |

Examples:

```bash
# Find all references to a symbol
tsa symbols ./src/ -n CONFIG_PATH
```
