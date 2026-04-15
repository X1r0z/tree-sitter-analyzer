# JSON Output Formats

All commands output structured JSON by default. Every successful response shares a common envelope:

```json
{
  "meta": {
    "root": "<analyzed path>",
    "files": <number>,
    "count": <number of results>
  },
  "results": [
    ...command-specific items...
  ]
}
```

`meta.files` is the number of files considered for the request.
Any `file` or `location.file` value in result items is reported relative to `meta.root`.

Errors are returned as:

```json
{
  "error": "<message>"
}
```

## `index`

Builds or rebuilds `tsa.db` in the current working directory.

| Field | Type | Description |
|-------|------|-------------|
| `database` | string | Output database path |
| `candidates` | int | Number of candidate source files discovered |
| `indexed` | int | Number of files currently indexed |
| `reparsed` | int | Number of files reparsed during this sync |
| `failed` | int | Number of files that failed to index |
| `errors` | string[] | Per-file indexing failures |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 1
  },
  "results": [
    {
      "database": "/current/working/directory/tsa.db",
      "candidates": 42,
      "indexed": 42,
      "reparsed": 3,
      "failed": 0,
      "errors": []
    }
  ]
}
```

## `functions`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Function/method name |
| `location` | object | Function/method location |
| `class_name` | string | Parent class (if method) |
| `params` | object[] | Function/method parameters |

Each `location` object contains:

| Field | Type | Description |
|-------|------|-------------|
| `file` | string | File path relative to `meta.root` |
| `start_line` | int | Start line |
| `end_line` | int | End line |

Each `params` item contains:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Parameter name |
| `type` | string\|null | Parameter type if available |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "name": "main",
      "location": {
        "file": "src/app.py",
        "start_line": 1,
        "end_line": 24
      },
      "params": [
        {"name": "config_path", "type": "str"},
        {"name": "verbose", "type": null}
      ]
    },
    {
      "name": "connect",
      "class_name": "Database",
      "location": {
        "file": "src/db.py",
        "start_line": 10,
        "end_line": 55
      },
      "params": [
        {"name": "dsn", "type": "str"},
        {"name": "timeout", "type": "int"}
      ]
    }
  ]
}
```

## `classes`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Class/struct/interface name |
| `kind` | string | Result kind |
| `location` | object | Class location |
| `methods` | string[] | Method names |
| `fields` | string[] | Field names |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 1
  },
  "results": [
    {
      "name": "Database",
      "kind": "class",
      "location": {
        "file": "src/db.py",
        "start_line": 1,
        "end_line": 120
      },
      "methods": ["__init__", "connect", "close"],
      "fields": ["dsn", "timeout"]
    }
  ]
}
```

## `fields`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Field name |
| `type` | string | Type annotation (if available) |
| `location` | object | Field location |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "name": "dsn",
      "type": "str",
      "location": {
        "file": "src/db.py",
        "start_line": 5,
        "end_line": 5
      }
    },
    {
      "name": "timeout",
      "type": "int",
      "location": {
        "file": "src/db.py",
        "start_line": 6,
        "end_line": 6
      }
    }
  ]
}
```

## `imports`

| Field | Type | Description |
|-------|------|-------------|
| `module` | string | Module name |
| `location` | object | Import location |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "module": "os",
      "location": {
        "file": "src/app.py",
        "start_line": 1,
        "end_line": 1
      }
    },
    {
      "module": "json",
      "location": {
        "file": "src/app.py",
        "start_line": 2,
        "end_line": 2
      }
    }
  ]
}
```

## `annotations`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Annotation or decorator name without the leading `@` |
| `source` | string | Full annotation/decorator text as it appears in source |
| `location` | object | Annotation location |
| `target` | object | Annotated/decorated target |

Each `target` object contains:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Name of the annotated/decorated class, function, or method |
| `kind` | string | Target kind, such as `class`, `function`, or `method` |
| `source` | string | Target declaration/signature text |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "name": "dataclass",
      "source": "@dataclass",
      "location": {
        "file": "src/models.py",
        "start_line": 3,
        "end_line": 3
      },
      "target": {
        "name": "User",
        "kind": "class",
        "source": "class User:"
      }
    },
    {
      "name": "Transactional",
      "source": "@Transactional(readOnly = true)",
      "location": {
        "file": "src/service/UserService.java",
        "start_line": 12,
        "end_line": 12
      },
      "target": {
        "name": "findUser",
        "kind": "method",
        "source": "public User findUser(String id)"
      }
    }
  ]
}
```

## `super-classes`

Result items use the same shape as `classes`.

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Parent class name |
| `kind` | string | Result kind |
| `location` | object | Class location |
| `methods` | string[] | Method names |
| `fields` | string[] | Field names |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "name": "User",
      "kind": "class",
      "location": {
        "file": "src/models.py",
        "start_line": 1,
        "end_line": 80
      },
      "methods": ["__init__", "save"],
      "fields": ["id", "email"]
    }
  ]
}
```

## `sub-classes`

Result items use the same shape as `classes`.

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Child class name |
| `kind` | string | Result kind |
| `location` | object | Class location |
| `methods` | string[] | Method names |
| `fields` | string[] | Field names |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "name": "AdminUser",
      "kind": "class",
      "location": {
        "file": "src/models.py",
        "start_line": 82,
        "end_line": 140
      },
      "methods": ["has_permission"],
      "fields": ["role"]
    }
  ]
}
```

## `callers`

| Field | Type | Description |
|-------|------|-------------|
| `caller` | string | Calling function name |
| `location` | object | Location of the call site |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "caller": "main",
      "location": {
        "file": "src/app.py",
        "start_line": 12,
        "end_line": 12
      }
    },
    {
      "caller": "handle_request",
      "location": {
        "file": "src/api.py",
        "start_line": 88,
        "end_line": 88
      }
    }
  ]
}
```

## `callees`

| Field | Type | Description |
|-------|------|-------------|
| `callee` | string | Called function name |
| `location` | object | Location of the call site |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "callee": "User",
      "location": {
        "file": "src/app.py",
        "start_line": 15,
        "end_line": 15
      }
    },
    {
      "callee": "greet",
      "location": {
        "file": "src/app.py",
        "start_line": 16,
        "end_line": 16
      }
    }
  ]
}
```

## `graph`

| Field | Type | Description |
|-------|------|-------------|
| `depth` | int | Actual depth of this call chain (number of edges) |
| `stacktrace` | string[] | Human-readable call chain frames |
| `nodes` | object[] | Ordered list of function nodes in this chain |

Each `nodes` item:

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Function name |
| `class_name` | string | Parent class (if method) |
| `location` | object | Function location |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "depth": 2,
      "stacktrace": [
        "save_result(db.py:45)",
        "transform(transform.py:12)",
        "process_data(app.py:30)"
      ],
      "nodes": [
        {
          "name": "process_data",
          "location": {
            "file": "src/app.py",
            "start_line": 28,
            "end_line": 40
          }
        },
        {
          "name": "transform",
          "location": {
            "file": "src/transform.py",
            "start_line": 10,
            "end_line": 20
          }
        },
        {
          "name": "save_result",
          "class_name": "Database",
          "location": {
            "file": "src/db.py",
            "start_line": 42,
            "end_line": 50
          }
        }
      ]
    }
  ]
}
```

## `definition`

Result items use the same shape as `functions`, plus:

| Field | Type | Description |
|-------|------|-------------|
| `source` | string | Full source code |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "name": "main",
      "location": {
        "file": "src/app.py",
        "start_line": 10,
        "end_line": 30
      },
      "params": [
        {"name": "config_path", "type": "str"},
        {"name": "retries", "type": null}
      ],
      "source": "def main():\n    config = load_config(CONFIG_PATH)\n    return process_data(config)\n"
    }
  ]
}
```

## `refs`

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Referenced symbol name |
| `kind` | string | Node type (e.g. `identifier`) |
| `location` | object | Reference location |
| `source` | string | Surrounding source line |

```json
{
  "meta": {
    "root": "/path/to/project",
    "files": 42,
    "count": 2
  },
  "results": [
    {
      "name": "CONFIG_PATH",
      "kind": "identifier",
      "location": {
        "file": "src/config.py",
        "start_line": 3,
        "end_line": 3
      },
      "source": "CONFIG_PATH = \"/etc/myapp/config.json\""
    },
    {
      "name": "CONFIG_PATH",
      "kind": "identifier",
      "location": {
        "file": "src/app.py",
        "start_line": 18,
        "end_line": 18
      },
      "source": "load_config(CONFIG_PATH)"
    }
  ]
}
```
