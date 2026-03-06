# JSON Output Formats

All commands output structured JSON by default. Every response shares a common envelope:

```json
{
  "path": "<analyzed path>",
  "files_searched": <number>,
  "count": <number of results>,
  ...command-specific array...
}
```

For cached queries, `files_searched` is the number of indexed files considered for the request. For uncached queries, it is the number of source files scanned directly.

---

## `index`

Builds or rebuilds `tsa.db` in the current working directory.

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Indexed project path |
| `database` | string | Output database path |
| `files_discovered` | int | Number of candidate source files found |
| `indexed_files` | int | Number of files successfully indexed |
| `failed_files` | int | Number of files that failed to index |
| `errors` | string[] | Per-file indexing failures |

```json
{
  "path": "/path/to/project",
  "database": "/current/working/directory/tsa.db",
  "files_discovered": 42,
  "indexed_files": 42,
  "failed_files": 0,
  "errors": []
}
```

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
  "count": 2,
  "class_name": "Database",
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
  "count": 1,
  "class_name": "AdminUser",
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
  "count": 1,
  "class_name": "User",
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
  "count": 2,
  "function": "process_data",
  "class_name": null,
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
  "count": 2,
  "function": "main",
  "class_name": null,
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
  "count": 1,
  "class_name": null,
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
  "count": 2,
  "name": "CONFIG_PATH",
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
