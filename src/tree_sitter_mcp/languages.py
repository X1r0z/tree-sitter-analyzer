"""Language support and parser management for tree-sitter."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path

import tree_sitter
import tree_sitter_go
import tree_sitter_java
import tree_sitter_javascript
import tree_sitter_python
import tree_sitter_typescript

LANGUAGE_MODULES: dict[str, Callable[[], tree_sitter.Language]] = {
    "python": lambda: tree_sitter.Language(tree_sitter_python.language()),
    "javascript": lambda: tree_sitter.Language(tree_sitter_javascript.language()),
    "typescript": lambda: tree_sitter.Language(tree_sitter_typescript.language_typescript()),
    "tsx": lambda: tree_sitter.Language(tree_sitter_typescript.language_tsx()),
    "java": lambda: tree_sitter.Language(tree_sitter_java.language()),
    "go": lambda: tree_sitter.Language(tree_sitter_go.language()),
}

FILE_EXTENSION_MAP: dict[str, str] = {
    ".py": "python",
    ".pyw": "python",
    ".pyi": "python",
    ".js": "javascript",
    ".mjs": "javascript",
    ".cjs": "javascript",
    ".jsx": "javascript",
    ".ts": "typescript",
    ".tsx": "tsx",
    ".java": "java",
    ".go": "go",
}


@dataclass
class LanguageInfo:
    name: str
    extensions: list[str]
    function_query: str
    class_query: str
    call_query: str
    import_query: str
    variable_query: str
    string_query: str
    field_query: str


LANGUAGE_QUERIES: dict[str, LanguageInfo] = {
    "python": LanguageInfo(
        name="python",
        extensions=[".py", ".pyw", ".pyi"],
        function_query="[(function_definition name: (identifier) @name) @function (decorated_definition definition: (function_definition name: (identifier) @name)) @function]",
        class_query="[(class_definition name: (identifier) @name) @class (decorated_definition definition: (class_definition name: (identifier) @name)) @class]",
        call_query="[(call function: (identifier) @callee) (call function: (attribute object: (_) @object attribute: (identifier) @method))] @call",
        import_query="[(import_statement name: (dotted_name) @module) (import_from_statement module_name: (dotted_name) @module) (import_from_statement module_name: (relative_import) @module)] @import",
        variable_query="[(assignment left: (identifier) @name) (assignment left: (pattern_list (identifier) @name))] @assignment",
        string_query="(string) @string",
        field_query="(class_definition body: (block (expression_statement (assignment left: (identifier) @name type: (type)? @type)))) @field",
    ),
    "javascript": LanguageInfo(
        name="javascript",
        extensions=[".js", ".mjs", ".cjs", ".jsx"],
        function_query="[(function_declaration name: (identifier) @name) (method_definition name: (property_identifier) @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function",
        class_query="[(class_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class",
        call_query="[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: (property_identifier) @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: (property_identifier) @method))] @call",
        import_query="[(import_statement source: (string) @module) (export_statement source: (string) @module) (call_expression function: (identifier) @callee arguments: (arguments (string) @module))] @import",
        variable_query="[(variable_declarator name: (identifier) @name) (variable_declarator name: (object_pattern (shorthand_property_identifier_pattern) @name)) (variable_declarator name: (array_pattern (identifier) @name)) (assignment_expression left: (identifier) @name)] @declaration",
        string_query="[(string) (template_string)] @string",
        field_query="(field_definition property: [(property_identifier) (private_property_identifier)] @name) @field",
    ),
    "typescript": LanguageInfo(
        name="typescript",
        extensions=[".ts"],
        function_query="[(function_declaration name: (identifier) @name) (method_definition name: (property_identifier) @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function",
        class_query="[(class_declaration name: (type_identifier) @name) (interface_declaration name: (type_identifier) @name) (type_alias_declaration name: (type_identifier) @name) (enum_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class",
        call_query="[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: (property_identifier) @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: (property_identifier) @method))] @call",
        import_query="[(import_statement source: (string) @module) (export_statement source: (string) @module)] @import",
        variable_query="[(variable_declarator name: (identifier) @name) (variable_declarator name: (object_pattern (shorthand_property_identifier_pattern) @name)) (variable_declarator name: (array_pattern (identifier) @name)) (assignment_expression left: (identifier) @name)] @declaration",
        string_query="[(string) (template_string)] @string",
        field_query="[(public_field_definition name: [(property_identifier) (private_property_identifier)] @name type: (type_annotation)? @type) (property_signature name: (property_identifier) @name type: (type_annotation)? @type)] @field",
    ),
    "tsx": LanguageInfo(
        name="tsx",
        extensions=[".tsx"],
        function_query="[(function_declaration name: (identifier) @name) (method_definition name: (property_identifier) @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function",
        class_query="[(class_declaration name: (type_identifier) @name) (interface_declaration name: (type_identifier) @name) (type_alias_declaration name: (type_identifier) @name) (enum_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class",
        call_query="[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: (property_identifier) @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: (property_identifier) @method))] @call",
        import_query="[(import_statement source: (string) @module) (export_statement source: (string) @module)] @import",
        variable_query="[(variable_declarator name: (identifier) @name) (variable_declarator name: (object_pattern (shorthand_property_identifier_pattern) @name)) (variable_declarator name: (array_pattern (identifier) @name)) (assignment_expression left: (identifier) @name)] @declaration",
        string_query="[(string) (template_string)] @string",
        field_query="[(public_field_definition name: [(property_identifier) (private_property_identifier)] @name type: (type_annotation)? @type) (property_signature name: (property_identifier) @name type: (type_annotation)? @type)] @field",
    ),
    "java": LanguageInfo(
        name="java",
        extensions=[".java"],
        function_query="[(method_declaration name: (identifier) @name) (constructor_declaration name: (identifier) @name)] @function",
        class_query="[(class_declaration name: (identifier) @name) (interface_declaration name: (identifier) @name) (enum_declaration name: (identifier) @name) (record_declaration name: (identifier) @name) (annotation_type_declaration name: (identifier) @name)] @class",
        call_query="[(method_invocation name: (identifier) @callee) (method_invocation object: (_) @object name: (identifier) @method) (object_creation_expression type: (_) @callee)] @call",
        import_query="[(import_declaration (scoped_identifier) @module) (import_declaration (identifier) @module)] @import",
        variable_query="(variable_declarator name: (identifier) @name) @declaration",
        string_query="(string_literal) @string",
        field_query="(field_declaration type: (_) @type declarator: (variable_declarator name: (identifier) @name)) @field",
    ),
    "go": LanguageInfo(
        name="go",
        extensions=[".go"],
        function_query="[(function_declaration name: (identifier) @name) (method_declaration name: (field_identifier) @name)] @function",
        class_query="(type_declaration (type_spec name: (type_identifier) @name type: [(struct_type) (interface_type)])) @class",
        call_query="[(call_expression function: (identifier) @callee) (call_expression function: (selector_expression operand: (_) @object field: (field_identifier) @method))] @call",
        import_query="(import_spec path: [(interpreted_string_literal) (raw_string_literal)] @module) @import",
        variable_query="[(short_var_declaration left: (expression_list (identifier) @name)) (var_spec name: (identifier) @name)] @declaration",
        string_query="[(interpreted_string_literal) (raw_string_literal)] @string",
        field_query="(field_declaration name: (field_identifier) @name type: (_) @type) @field",
    ),
}


def detect_language(file_path: str | Path) -> str | None:
    ext = Path(file_path).suffix.lower()
    return FILE_EXTENSION_MAP.get(ext)


@lru_cache(maxsize=16)
def get_language(language: str) -> tree_sitter.Language | None:
    if language not in LANGUAGE_MODULES:
        return None
    return LANGUAGE_MODULES[language]()


def get_parser(language: str) -> tree_sitter.Parser | None:
    lang = get_language(language)
    if not lang:
        return None
    parser = tree_sitter.Parser()
    parser.language = lang
    return parser


def get_language_info(language: str) -> LanguageInfo | None:
    return LANGUAGE_QUERIES.get(language)


def get_supported_languages() -> list[str]:
    return list(LANGUAGE_MODULES.keys())
