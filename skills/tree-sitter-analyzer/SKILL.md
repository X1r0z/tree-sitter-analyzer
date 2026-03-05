---
name: "tree-sitter-analyzer"
description: >
  Structural code analysis via tree-sitter AST (functions, classes, imports, call graph, inheritance, symbol references).
  Use for "who calls X?", "what does X call?", "where is X defined?", "list functions/classes", "class fields/methods",
  "subclasses/superclasses", "find symbol references", and any structural code understanding across files or directories.
  Prefer over Grep for code understanding — Grep is for exact literal text matches only.
  Supports Python, JavaScript/TypeScript, Java, Go.
allowed-tools: Bash(tree-sitter-analyzer:*)
---

# Tree-sitter Analyzer

AST-based code structure analysis across Python, JavaScript/TypeScript, Java, and Go.

## When to Use

**Always prefer tree-sitter-analyzer over Grep/finder for code understanding tasks.**

tree-sitter-analyzer parses the actual AST (Abstract Syntax Tree) of source code, not plain text. This means it understands code structure — it can distinguish a function definition from a function call, a class field from a local variable, and a method override from a standalone function. Grep treats code as flat text and will miss structural relationships, produce false matches on comments/strings, and break across formatting differences. tree-sitter-analyzer gives precise, semantically meaningful results regardless of whitespace, indentation style, or code formatting.

### Structural Code Understanding

Use this skill whenever you need answers about code structure or relationships:

- **Call graph analysis** — "Who calls this function?", "What does this function call?", "Trace the call chain from entry point to sink"
- **Definition lookup** — "Where is this function/class defined?", "Show me the source code of X"
- **Class structure** — "What fields/methods does this class have?", "What are the subclasses/superclasses?"
- **Symbol tracking** — "Find all references to this identifier across the project"
- **Inventory** — "List all functions/classes/imports in this directory"
- **Impact analysis** — "If I change this function, what else is affected?"

### Code Auditing & Security Review

tree-sitter-analyzer is particularly powerful for security audits because it can trace data flow structurally rather than relying on string matching:

- **Dangerous function inventory** — Find all calls to security-sensitive functions (e.g., `eval`, `exec`, `os.system`, `subprocess.Popen`, `Runtime.exec`, `sql.Query`) by listing callees or searching symbols, then trace their callers to understand input sources
- **Taint source tracing** — Identify functions that handle user input (e.g., `request.GET`, `req.body`, `Scanner.nextLine`), then use `callers` to trace how tainted data propagates through the codebase
- **Sink reachability** — Start from a dangerous sink function, use `callers` recursively to build the call chain back to entry points and determine if user-controlled data can reach it
- **Attack surface mapping** — Use `functions` + `classes` to inventory all public API endpoints, handlers, and entry points; then use `callees` to map what internal functions each endpoint reaches
- **Hardcoded secrets/config** — Use `function-strings` to extract all string literals inside sensitive functions (e.g., database connection, authentication) to find hardcoded credentials, keys, or tokens
- **Privilege analysis** — Use `sub-classes` to find all implementations of permission/auth base classes; use `fields` to inspect their configuration
- **Dependency mapping** — Use `imports` to audit which modules import dangerous libraries; use `symbols` to find every reference to security-critical variables

### Codebase Onboarding & Refactoring

- **New codebase exploration** — Quickly inventory the architecture: list all classes, map inheritance hierarchies, identify entry points
- **Refactoring planning** — Before renaming or moving a function, use `callers` + `symbols` to find every reference that needs updating
- **Dead code detection** — Find functions with zero callers to identify potentially unused code
- **Architecture documentation** — Extract class hierarchies and call graphs to generate codebase maps

### When to Fall Back to Grep

Only use Grep instead of tree-sitter-analyzer when:
- You need an exact literal string match (e.g., a specific error message, URL, or log text)
- You're searching in comments, documentation, or non-code files (`.md`, `.txt`, `.yaml`, etc.)
- The target language is not supported (only Python, JS/TS, Java, Go are supported)

## Quick Command Chooser

| Question | Command |
|----------|---------|
| List all functions/methods | `functions <path>` |
| List all classes/interfaces | `classes <path>` |
| What fields does class X have? | `fields <path> -c X` |
| What does function X call? | `callees <path> -f X` |
| Who calls function X? | `callers <path> -f X` |
| Get function source code | `definition <path> -f X` |
| List imports / find a module | `imports <path> [-q pattern]` |
| List variables | `variables <path> [-q pattern]` |
| Find all references to symbol | `symbols <path> -n NAME` |
| Parent classes of X | `super-classes <path> -c X` |
| Child classes of X | `sub-classes <path> -c X` |
| Variables inside function X | `function-variables <path> -f X` |
| String literals inside function X | `function-strings <path> -f X` |

## Installation

```bash
cargo install --git https://github.com/X1r0z/tree-sitter-analyzer
```

## Usage

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

Default output is human-readable. Add `--json` for structured JSON output.

When using `--json`, pipe through `jq` to reduce context and extract only what you need:

```bash
# Names only
tree-sitter-analyzer functions ./src/ --json | jq -r '.functions[].name'

# Location tuples
tree-sitter-analyzer classes ./src/ --json | jq -r '.classes[] | "\(.name)\t\(.file):\(.start_line)"'

# Caller summary
tree-sitter-analyzer callers ./src/ -f process_data --json | jq -r '.callers[] | "\(.caller)\t\(.file):\(.line)"'
```

Full JSON output schemas for each command: see `references/OUTPUT.md`.

## Commands

### `functions` — Extract function/method definitions

```bash
tree-sitter-analyzer functions <path> [-q QUERY] [--body] [--json]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by name (regex) |
| `--body` | Include function body |

```bash
tree-sitter-analyzer functions ./src/ -q "^get_" --json
```

### `classes` — Extract class/struct/interface definitions

```bash
tree-sitter-analyzer classes <path> [-q QUERY] [--json]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by name (regex) |

```bash
tree-sitter-analyzer classes ./src/ -q "Service$" --json
```

### `fields` — Get class fields

```bash
tree-sitter-analyzer fields <path> -c CLASS_NAME [--json]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name (required) |

```bash
tree-sitter-analyzer fields ./src/ -c DatabaseConfig --json
```

### `imports` — Extract import statements

```bash
tree-sitter-analyzer imports <path> [-q QUERY] [--json]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by module name (regex) |

```bash
tree-sitter-analyzer imports ./src/ -q "^(os|sys)$" --json
```

### `variables` — Extract variable declarations

```bash
tree-sitter-analyzer variables <path> [-q QUERY] [--json]
```

| Option | Description |
|--------|-------------|
| `-q, --query` | Filter by name (regex) |

```bash
tree-sitter-analyzer variables ./src/ -q "^[A-Z_]+$" --json
```

### `callers` — Find who calls a function

```bash
tree-sitter-analyzer callers <path> -f FUNCTION [-c CLASS_NAME] [--json]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name (required) |
| `-c, --class-name` | Scope to a method in this class |

```bash
tree-sitter-analyzer callers ./src/ -f save -c DatabaseHandler --json
```

### `callees` — Find what a function calls

```bash
tree-sitter-analyzer callees <path> -f FUNCTION [-c CLASS_NAME] [--json]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name (required) |
| `-c, --class-name` | Scope to a method in this class |

```bash
tree-sitter-analyzer callees ./src/ -f initialize -c Application --json
```

### `definition` — Get function source code

```bash
tree-sitter-analyzer definition <path> -f FUNCTION [-c CLASS_NAME] [--json]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name (required) |
| `-c, --class-name` | Scope to a method in this class |

```bash
tree-sitter-analyzer definition ./src/ -f parse_config --json
```

### `function-variables` — Variables inside a function

```bash
tree-sitter-analyzer function-variables <path> -f FUNCTION [-c CLASS_NAME] [--json]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name (required) |
| `-c, --class-name` | Scope to a method in this class |

```bash
tree-sitter-analyzer function-variables ./src/ -f process_request --json
```

### `function-strings` — String literals inside a function

```bash
tree-sitter-analyzer function-strings <path> -f FUNCTION [-c CLASS_NAME] [--json]
```

| Option | Description |
|--------|-------------|
| `-f, --function` | Function name (required) |
| `-c, --class-name` | Scope to a method in this class |

```bash
tree-sitter-analyzer function-strings ./src/ -f handle_request --json
```

### `symbols` — Find all references to an identifier

```bash
tree-sitter-analyzer symbols <path> -n NAME [--json]
```

| Option | Description |
|--------|-------------|
| `-n, --name` | Identifier name (required) |

```bash
tree-sitter-analyzer symbols ./src/ -n CONFIG_PATH --json
```

### `super-classes` — Get parent classes

```bash
tree-sitter-analyzer super-classes <path> -c CLASS_NAME [--json]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name (required) |

```bash
tree-sitter-analyzer super-classes ./src/ -c AdminUser --json
```

### `sub-classes` — Get child classes

```bash
tree-sitter-analyzer sub-classes <path> -c CLASS_NAME [--json]
```

| Option | Description |
|--------|-------------|
| `-c, --class-name` | Class name (required) |

```bash
tree-sitter-analyzer sub-classes ./src/ -c BaseModel --json
```

## Typical Workflows

### Inventory a repository

```bash
tree-sitter-analyzer functions /path/to/project --json | jq -r '.functions[] | "\(.name)\t\(.file):\(.start_line)"'
tree-sitter-analyzer classes /path/to/project --json | jq -r '.classes[] | "\(.name)\t\(.file):\(.start_line)"'
tree-sitter-analyzer imports /path/to/project --json | jq -r '.imports[] | "\(.module)\t\(.file):\(.line)"'
```

### Impact analysis (who calls X / what does X call)

```bash
tree-sitter-analyzer callers /path/to/project -f process_data --json
tree-sitter-analyzer callees /path/to/project -f process_data --json
```

### Trace a symbol through a project

```bash
tree-sitter-analyzer symbols /path/to/project -n CONFIG_PATH --json
```

### Inspect a function in detail

```bash
tree-sitter-analyzer definition /path/to/project -f main --json
tree-sitter-analyzer function-variables /path/to/project -f main --json
tree-sitter-analyzer function-strings /path/to/project -f main --json
```

### Map class hierarchy

```bash
tree-sitter-analyzer super-classes /path/to/project -c AdminUser --json
tree-sitter-analyzer sub-classes /path/to/project -c BaseModel --json
```
