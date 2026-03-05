# JSON Output Formats

All commands support `--json` for structured output. Every response shares a common envelope:

```json
{
  "path": "<analyzed path>",
  "files_searched": <number>,
  "count": <number of results>,
  ...command-specific array...
}
```

---

## `functions`

Array key: `functions`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Function/method name |
| `start_line` | int | Start line |
| `end_line` | int | End line |
| `file` | string | File path |
| `is_method` | bool | Present if it's a class method |
| `class_name` | string | Parent class (if method) |
| `body` | string | Function body (only with `--body`) |

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

## `classes`

Array key: `classes`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Class/struct/interface name |
| `start_line` | int | Start line |
| `end_line` | int | End line |
| `methods` | string[] | Method names |
| `fields` | string[] | Field names |
| `file` | string | File path |

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
      "methods": ["__init__", "connect", "close"],
      "fields": ["dsn", "timeout"],
      "file": "/path/to/project/src/db.py"
    }
  ]
}
```

## `fields`

Array key: `fields`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Field name |
| `line` | int | Line number |
| `file` | string | File path |
| `type` | string | Type annotation (if available) |
| `class_name` | string | Owning class |

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

## `imports`

Array key: `imports`

| Field | Type | Description |
|-------|------|-------------|
| `module` | string | Module name |
| `line` | int | Line number |
| `file` | string | File path |

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "count": 2,
  "imports": [
    {"module": "os", "line": 1, "file": "/path/to/project/src/app.py"},
    {"module": "json", "line": 2, "file": "/path/to/project/src/app.py"}
  ]
}
```

## `variables`

Array key: `variables`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Variable name |
| `line` | int | Line number |
| `scope` | string\|null | Enclosing function (null = module-level) |
| `file` | string | File path |

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "count": 2,
  "variables": [
    {"name": "CONFIG_PATH", "line": 3, "scope": null, "file": "/path/to/project/src/config.py"},
    {"name": "payload", "line": 18, "scope": "main", "file": "/path/to/project/src/app.py"}
  ]
}
```

## `super-classes`

Array key: `super_classes`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Parent class name |
| `start_line` | int | Start line |
| `end_line` | int | End line |
| `methods` | string[] | Method names |
| `fields` | string[] | Field names |
| `file` | string | File path |

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
      "methods": ["__init__", "save"],
      "fields": ["id", "email"],
      "file": "/path/to/project/src/models.py"
    }
  ]
}
```

## `sub-classes`

Array key: `sub_classes`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Child class name |
| `start_line` | int | Start line |
| `end_line` | int | End line |
| `methods` | string[] | Method names |
| `fields` | string[] | Field names |
| `file` | string | File path |

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
      "methods": ["has_permission"],
      "fields": ["role"],
      "file": "/path/to/project/src/models.py"
    }
  ]
}
```

## `callers`

Array key: `callers`

| Field | Type | Description |
|-------|------|-------------|
| `caller` | string | Calling function name |
| `line` | int | Line of the call |
| `file` | string | File path |
| `target_class` | string\|null | Class of the callee (if method) |

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "function": "process_data",
  "class_name": null,
  "count": 2,
  "callers": [
    {"caller": "main", "line": 12, "file": "/path/to/project/src/app.py", "target_class": null},
    {"caller": "handle_request", "line": 88, "file": "/path/to/project/src/api.py", "target_class": null}
  ]
}
```

## `callees`

Array key: `callees`

| Field | Type | Description |
|-------|------|-------------|
| `callee` | string | Called function name |
| `line` | int | Line of the call |
| `file` | string | File path |
| `class_name` | string\|null | Class scope (if method) |

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "function": "main",
  "class_name": null,
  "count": 2,
  "callees": [
    {"callee": "load_config", "line": 18, "file": "/path/to/project/src/app.py", "class_name": null},
    {"callee": "process_data", "line": 25, "file": "/path/to/project/src/app.py", "class_name": null}
  ]
}
```

## `definition`

Array key: `functions`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Function name |
| `start_line` | int | Start line |
| `end_line` | int | End line |
| `file` | string | File path |
| `body` | string | Full source code |

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
      "body": "def main():\n    config = load_config(CONFIG_PATH)\n    return process_data(config)\n"
    }
  ]
}
```

## `function-variables`

Array key: `variables`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Variable name |
| `line` | int | Line number |
| `file` | string | File path |

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "function": "main",
  "class_name": null,
  "count": 2,
  "variables": [
    {"name": "config", "line": 11, "file": "/path/to/project/src/app.py"},
    {"name": "result", "line": 12, "file": "/path/to/project/src/app.py"}
  ]
}
```

## `function-strings`

Array key: `strings`

| Field | Type | Description |
|-------|------|-------------|
| `value` | string | String literal content |
| `line` | int | Line number |
| `file` | string | File path |

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "function": "main",
  "class_name": null,
  "count": 1,
  "strings": [
    {"value": "starting...", "line": 15, "file": "/path/to/project/src/app.py"}
  ]
}
```

## `symbols`

Array key: `references`

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Node type (e.g. "identifier") |
| `location.file` | string | File path |
| `location.start_line` | int | Start line |
| `location.end_line` | int | End line |
| `context` | string | Surrounding source line |

```json
{
  "path": "/path/to/project",
  "files_searched": 42,
  "name": "CONFIG_PATH",
  "count": 2,
  "references": [
    {
      "type": "identifier",
      "location": {"file": "/path/to/project/src/config.py", "start_line": 3, "end_line": 3},
      "context": "CONFIG_PATH = \"/etc/myapp/config.json\""
    },
    {
      "type": "identifier",
      "location": {"file": "/path/to/project/src/app.py", "start_line": 18, "end_line": 18},
      "context": "load_config(CONFIG_PATH)"
    }
  ]
}
```
