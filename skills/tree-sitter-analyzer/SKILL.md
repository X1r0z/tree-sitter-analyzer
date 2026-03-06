---
name: "tree-sitter-analyzer"
description: >
  Structural code analysis via tree-sitter AST (functions, classes, imports, call graph, inheritance, symbol references).
  Use for "who calls X?", "what does X call?", "where is X defined?", "list functions/classes", "class fields/methods",
  "subclasses/superclasses", "find symbol references", and any structural code understanding across files or directories.
  Prefer over Grep for code understanding — Grep is for exact literal text matches only.
  Supports Python, JavaScript/TypeScript, Java, Go.
allowed-tools: Bash(tsa:*)
---

# Tree-sitter Analyzer

AST-based code structure analysis across Python, JavaScript/TypeScript, Java, and Go.

## When to Use

**Always prefer tree-sitter-analyzer (tsa) over Grep/finder for code understanding tasks.**

tree-sitter-analyzer (tsa) parses the actual AST (Abstract Syntax Tree) of source code, not plain text. This means it understands code structure — it can distinguish a function definition from a function call, a class field from a local variable, and a method override from a standalone function. Grep treats code as flat text and will miss structural relationships, produce false matches on comments/strings, and break across formatting differences. tree-sitter-analyzer (tsa) gives precise, semantically meaningful results regardless of whitespace, indentation style, or code formatting.

### Structural Code Understanding

Use this skill whenever you need answers about code structure or relationships:

- **Call graph analysis** — "Who calls this function?", "What does this function call?", "Trace the call chain from entry point to sink"
- **Definition lookup** — "Where is this function/class defined?", "Show me the source code of X"
- **Class structure** — "What fields/methods does this class have?", "What are the subclasses/superclasses?"
- **Symbol tracking** — "Find all references to this identifier across the project"
- **Inventory** — "List all functions/classes/imports in this directory"
- **Impact analysis** — "If I change this function, what else is affected?"

### Code Auditing & Security Review

tree-sitter-analyzer (tsa) is particularly powerful for security audits because it can trace data flow structurally rather than relying on string matching:

- **Dangerous function inventory** — Find all calls to security-sensitive functions (e.g., `eval`, `exec`, `os.system`, `subprocess.Popen`, `Runtime.exec`, `sql.Query`) by listing callees or searching symbols, then trace their callers to understand input sources
- **Taint source tracing** — Identify functions that handle user input (e.g., `request.GET`, `req.body`, `Scanner.nextLine`), then use `callers` to trace how tainted data propagates through the codebase
- **Sink reachability** — Start from a dangerous sink function, use `callers` recursively to build the call chain back to entry points and determine if user-controlled data can reach it
- **Attack surface mapping** — Use `functions` + `classes` to inventory all public API endpoints, handlers, and entry points; then use `callees` to map what internal functions each endpoint reaches
- **Privilege analysis** — Use `sub-classes` to find all implementations of permission/auth base classes; use `fields` to inspect their configuration
- **Dependency mapping** — Use `imports` to audit which modules import dangerous libraries; use `symbols` to find every reference to security-critical identifiers

### Codebase Onboarding & Refactoring

- **New codebase exploration** — Quickly inventory the architecture: list all classes, map inheritance hierarchies, identify entry points
- **Refactoring planning** — Before renaming or moving a function, use `callers` + `symbols` to find every reference that needs updating
- **Dead code detection** — Find functions with zero callers to identify potentially unused code
- **Architecture documentation** — Extract class hierarchies and call graphs to generate codebase maps

### When to Fall Back to Grep

Only use Grep instead of tree-sitter-analyzer (tsa) when:
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
| Find all references to symbol | `symbols <path> -n NAME` |
| Parent classes of X | `super-classes <path> -c X` |
| Child classes of X | `sub-classes <path> -c X` |

## Installation

```bash
cargo install --git https://github.com/X1r0z/tree-sitter-analyzer
```

## Usage

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

Use `-l, --language <LANGUAGE>` to limit analysis to one language when the target directory mixes multiple supported languages. Allowed values: `python`, `java`, `go`, `javascript`, `typescript`, `tsx`.

### Output Formats

Default output is structured JSON.

Pipe through `jq` to reduce context and extract only what you need:

```bash
# Names only
tsa functions ./src/ -l python | jq -r '.functions[].name'

# Location tuples
tsa classes ./src/ | jq -r '.classes[] | "\(.name)\t\(.file):\(.start_line)"'

# Caller summary
tsa callers ./src/ -f process_data | jq -r '.callers[] | "\(.caller)\t\(.file):\(.line)"'
```

Full JSON output schemas for each command: see `references/OUTPUT.md`.

## Commands

### `functions` — Extract function/method definitions

```bash
tsa functions <path> [-l LANGUAGE] [-q QUERY] [--body]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-q, --query` | Filter by name (regex) |
| `--body` | Include function body |

```bash
tsa functions ./src/ -q "^get_"
```

### `classes` — Extract class/struct/interface definitions

```bash
tsa classes <path> [-l LANGUAGE] [-q QUERY]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-q, --query` | Filter by name (regex) |

```bash
tsa classes ./src/ -q "Service$"
```

### `fields` — Get class fields

```bash
tsa fields <path> [-l LANGUAGE] -c CLASS_NAME
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-c, --class-name` | Class name (required) |

```bash
tsa fields ./src/ -c DatabaseConfig
```

### `imports` — Extract import statements

```bash
tsa imports <path> [-l LANGUAGE] [-q QUERY]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-q, --query` | Filter by module name (regex) |

```bash
tsa imports ./src/ -q "^(os|sys)$"
```

### `callers` — Find who calls a function

```bash
tsa callers <path> [-l LANGUAGE] -f FUNCTION [-c CLASS_NAME]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-f, --function` | Function name (required) |
| `-c, --class-name` | Scope to a method in this class |

```bash
tsa callers ./src/ -f save -c DatabaseHandler
```

### `callees` — Find what a function calls

```bash
tsa callees <path> [-l LANGUAGE] -f FUNCTION [-c CLASS_NAME]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-f, --function` | Function name (required) |
| `-c, --class-name` | Scope to a method in this class |

```bash
tsa callees ./src/ -f initialize -c Application
```

### `definition` — Get function source code

```bash
tsa definition <path> [-l LANGUAGE] -f FUNCTION [-c CLASS_NAME]
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-f, --function` | Function name (required) |
| `-c, --class-name` | Scope to a method in this class |

```bash
tsa definition ./src/ -f parse_config
```

### `symbols` — Find all references to an identifier

```bash
tsa symbols <path> [-l LANGUAGE] -n NAME
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-n, --name` | Identifier name (required) |

```bash
tsa symbols ./src/ -n CONFIG_PATH
```

### `super-classes` — Get parent classes

```bash
tsa super-classes <path> [-l LANGUAGE] -c CLASS_NAME
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-c, --class-name` | Class name (required) |

```bash
tsa super-classes ./src/ -c AdminUser
```

### `sub-classes` — Get child classes

```bash
tsa sub-classes <path> [-l LANGUAGE] -c CLASS_NAME
```

| Option | Description |
|--------|-------------|
| `-l, --language` | Only analyze files for a single language |
| `-c, --class-name` | Class name (required) |

```bash
tsa sub-classes ./src/ -c BaseModel
```

## Typical Workflows

### Inventory a repository

```bash
tsa functions /path/to/project | jq -r '.functions[] | "\(.name)\t\(.file):\(.start_line)"'
tsa classes /path/to/project | jq -r '.classes[] | "\(.name)\t\(.file):\(.start_line)"'
tsa imports /path/to/project | jq -r '.imports[] | "\(.module)\t\(.file):\(.line)"'
```

### Impact analysis (who calls X / what does X call)

```bash
tsa callers /path/to/project -f process_data
tsa callees /path/to/project -f process_data
```

### Trace a symbol through a project

```bash
tsa symbols /path/to/project -n CONFIG_PATH
```

### Inspect a function in detail

```bash
tsa definition /path/to/project -f main
```

### Map class hierarchy

```bash
tsa super-classes /path/to/project -c AdminUser
tsa sub-classes /path/to/project -c BaseModel
```
