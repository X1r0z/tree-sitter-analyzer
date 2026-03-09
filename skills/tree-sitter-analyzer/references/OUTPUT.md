# JSON Output Formats

All commands output structured JSON by default. Every response shares a common envelope:

```json
{
  "path": "<analyzed path>",
  "searched_files": <number>,
  "count": <number of results>,
  ...command-specific array...
}
```

For cached queries, `searched_files` is the number of indexed files considered for the request. For uncached queries, it is the number of source files scanned directly.
Any `file` or `location.file` value in result items is reported relative to `path`.

---

## `index`

Builds or rebuilds `tsa.db` in the current working directory.

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Indexed project path |
| `database` | string | Output database path |
| `discovered_files` | int | Number of candidate source files found |
| `indexed_files` | int | Number of files successfully indexed |
| `failed_files` | int | Number of files that failed to index |
| `errors` | string[] | Per-file indexing failures |

```json
{
  "path": "/path/to/project",
  "database": "/current/working/directory/tsa.db",
  "discovered_files": 42,
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
| `file` | string | File path relative to `path` |
| `class_name` | string | Parent class (if method) |
| `params` | object[] | Function/method parameters |

Each `params` item contains:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Parameter name |
| `type` | string\|null | Parameter type if available |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 2,
  "functions": [
    {
      "name": "main",
      "start_line": 1,
      "end_line": 24,
      "file": "src/app.py",
      "params": [
        {"name": "config_path", "type": "str"},
        {"name": "verbose", "type": null}
      ]
    },
    {
      "name": "connect",
      "start_line": 10,
      "end_line": 55,
      "file": "src/db.py",
      "class_name": "Database",
      "params": [
        {"name": "dsn", "type": "str"},
        {"name": "timeout", "type": "int"}
      ]
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
| `file` | string | File path relative to `path` |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 1,
  "classes": [
    {
      "name": "Database",
      "start_line": 1,
      "end_line": 120,
      "methods": ["__init__", "connect", "close"],
      "fields": ["dsn", "timeout"],
      "file": "src/db.py"
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
| `file` | string | File path relative to `path` |
| `type` | string | Type annotation (if available) |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 2,
  "class_name": "Database",
  "fields": [
    {
      "name": "dsn",
      "line": 5,
      "file": "src/db.py",
      "type": "str"
    },
    {
      "name": "timeout",
      "line": 6,
      "file": "src/db.py",
      "type": "int"
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
| `file` | string | File path relative to `path` |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 2,
  "imports": [
    {"module": "os", "line": 1, "file": "src/app.py"},
    {"module": "json", "line": 2, "file": "src/app.py"}
  ]
}
```

## `annotations`

Array key: `annotations`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Annotation or decorator name without the leading `@` |
| `signature` | string | Full annotation/decorator text as it appears in source |
| `line` | int | Start line number |
| `file` | string | File path relative to `path` |
| `target_name` | string | Name of the annotated/decorated class, function, or method |
| `target_type` | string | Target kind, such as `class`, `function`, or `method` |
| `target_signature` | string | Target declaration/signature text |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 2,
  "annotations": [
    {
      "name": "dataclass",
      "signature": "@dataclass",
      "line": 3,
      "file": "src/models.py",
      "target_name": "User",
      "target_type": "class",
      "target_signature": "class User:"
    },
    {
      "name": "Transactional",
      "signature": "@Transactional(readOnly = true)",
      "line": 12,
      "file": "src/service/UserService.java",
      "target_name": "findUser",
      "target_type": "method",
      "target_signature": "public User findUser(String id)"
    }
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
| `file` | string | File path relative to `path` |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 1,
  "class_name": "AdminUser",
  "super_classes": [
    {
      "name": "User",
      "start_line": 1,
      "end_line": 80,
      "methods": ["__init__", "save"],
      "fields": ["id", "email"],
      "file": "src/models.py"
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
| `file` | string | File path relative to `path` |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 1,
  "class_name": "User",
  "sub_classes": [
    {
      "name": "AdminUser",
      "start_line": 82,
      "end_line": 140,
      "methods": ["has_permission"],
      "fields": ["role"],
      "file": "src/models.py"
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
| `file` | string | File path relative to `path` |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 2,
  "function": "process_data",
  "class_name": null,
  "callers": [
    {"caller": "main", "line": 12, "file": "src/app.py"},
    {"caller": "handle_request", "line": 88, "file": "src/api.py"}
  ]
}
```

## `callees`

Array key: `callees`

| Field | Type | Description |
|-------|------|-------------|
| `callee` | string | Called function name |
| `line` | int | Line of the call |
| `file` | string | File path relative to `path` |
| `class_name` | string\|null | Class scope (if method) |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 2,
  "function": "main",
  "class_name": null,
  "callees": [
    {"callee": "load_config", "line": 18, "file": "src/app.py", "class_name": null},
    {"callee": "process_data", "line": 25, "file": "src/app.py", "class_name": null}
  ]
}
```

## `graph`

Array key: `graphs`

Envelope includes extra metadata fields:

| Field | Type | Description |
|-------|------|-------------|
| `function` | string | The function name traced |
| `class_name` | string\|null | Class scope (if method) |
| `direction` | string | `"forward"` or `"backward"` |
| `max_depth` | int | Requested maximum depth |

Each `graphs` item:

| Field | Type | Description |
|-------|------|-------------|
| `depth` | int | Actual depth of this call chain (number of edges) |
| `stacktrace` | string[] | Human-readable call chain frames, ordered callee → caller (bottom-up) |
| `path` | object[] | Ordered list of function nodes in this chain |

Each `path` item:

| Field | Type | Description |
|-------|------|-------------|
| `file` | string | File path relative to `path` |
| `start_line` | int | Start line |
| `end_line` | int | End line |
| `name` | string | Function name |
| `class_name` | string | Parent class (if method, omitted if none) |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 2,
  "function": "process_data",
  "class_name": null,
  "direction": "forward",
  "max_depth": 3,
  "graphs": [
    {
      "depth": 2,
      "stacktrace": [
        "save_result(db.py:45)",
        "transform(transform.py:12)",
        "process_data(app.py:30)"
      ],
      "path": [
        {
          "file": "src/app.py",
          "start_line": 28,
          "end_line": 40,
          "name": "process_data"
        },
        {
          "file": "src/transform.py",
          "start_line": 10,
          "end_line": 20,
          "name": "transform"
        },
        {
          "file": "src/db.py",
          "start_line": 42,
          "end_line": 50,
          "name": "save_result",
          "class_name": "Database"
        }
      ]
    }
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
| `file` | string | File path relative to `path` |
| `params` | object[] | Function/method parameters |
| `body` | string | Full source code |

Each `params` item contains:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Parameter name |
| `type` | string\|null | Parameter type if available |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 1,
  "class_name": null,
  "functions": [
    {
      "name": "main",
      "start_line": 10,
      "end_line": 30,
      "file": "src/app.py",
      "params": [
        {"name": "config_path", "type": "str"},
        {"name": "retries", "type": null}
      ],
      "body": "def main():\n    config = load_config(CONFIG_PATH)\n    return process_data(config)\n"
    }
  ]
}
```

## `refs`

Array key: `references`

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Node type (e.g. "identifier") |
| `location.file` | string | File path relative to `path` |
| `location.start_line` | int | Start line |
| `location.end_line` | int | End line |
| `context` | string | Surrounding source line |

```json
{
  "path": "/path/to/project",
  "searched_files": 42,
  "count": 2,
  "name": "CONFIG_PATH",
  "refs": [
    {
      "type": "identifier",
      "location": {"file": "src/config.py", "start_line": 3, "end_line": 3},
      "context": "CONFIG_PATH = \"/etc/myapp/config.json\""
    },
    {
      "type": "identifier",
      "location": {"file": "src/app.py", "start_line": 18, "end_line": 18},
      "context": "load_config(CONFIG_PATH)"
    }
  ]
}
```
