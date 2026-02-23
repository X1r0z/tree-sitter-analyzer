"""Core code analysis functionality using tree-sitter."""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path

import tree_sitter

from .languages import detect_language, get_language, get_language_info, get_parser


@lru_cache(maxsize=128)
def _get_compiled_query(language: str, query_str: str) -> tree_sitter.Query | None:
    """Get a cached compiled query for the given language and query string."""
    lang = get_language(language)
    if not lang:
        return None
    try:
        return tree_sitter.Query(lang, query_str)
    except Exception as e:
        print(f"Error compiling query for {language}: {e}")
        return None


@dataclass
class Location:
    file: str
    start_line: int
    end_line: int

    def to_dict(self) -> dict:
        return {
            "file": self.file,
            "start_line": self.start_line,
            "end_line": self.end_line,
        }


@dataclass
class FunctionInfo:
    name: str
    location: Location
    parameters: str = ""
    body: str = ""
    is_method: bool = False
    class_name: str | None = None
    node: tree_sitter.Node | None = None

    def to_dict(self, include_body: bool = True, include_file: bool = True) -> dict:
        result = {
            "name": self.name,
            "start_line": self.location.start_line,
            "end_line": self.location.end_line,
        }
        if include_file:
            result["file"] = self.location.file
        if self.is_method:
            result["is_method"] = True
        if self.class_name:
            result["class_name"] = self.class_name
        if include_body and self.body:
            result["body"] = self.body
        return result


@dataclass
class ClassInfo:
    name: str
    location: Location
    methods: list[str] = field(default_factory=list)
    fields: list[str] = field(default_factory=list)
    super_classes: list[str] = field(default_factory=list)

    def to_dict(self, include_file: bool = True) -> dict:
        result = {
            "name": self.name,
            "start_line": self.location.start_line,
            "end_line": self.location.end_line,
            "methods": self.methods,
            "fields": self.fields,
        }
        if include_file:
            result["file"] = self.location.file
        return result


@dataclass
class CallInfo:
    callee: str
    location: Location
    caller: str | None = None
    caller_class_name: str | None = None
    object_name: str | None = None
    is_method_call: bool = False

    def to_dict(self, include_file: bool = True) -> dict:
        result = {
            "caller": self.caller,
            "callee": self.callee,
            "line": self.location.start_line,
        }
        if include_file:
            result["file"] = self.location.file
        if self.object_name:
            result["object"] = self.object_name
        if self.is_method_call:
            result["is_method_call"] = True
        return result


@dataclass
class VariableInfo:
    name: str
    location: Location
    scope: str | None = None

    def to_dict(self, include_file: bool = True) -> dict:
        result = {
            "name": self.name,
            "line": self.location.start_line,
            "scope": self.scope,
        }
        if include_file:
            result["file"] = self.location.file
        return result


@dataclass
class ImportInfo:
    module: str
    location: Location

    def to_dict(self, include_file: bool = True) -> dict:
        result = {
            "module": self.module,
            "line": self.location.start_line,
        }
        if include_file:
            result["file"] = self.location.file
        return result


@dataclass
class StringLiteral:
    value: str
    location: Location

    def to_dict(self, include_file: bool = True) -> dict:
        result = {
            "value": self.value,
            "line": self.location.start_line,
        }
        if include_file:
            result["file"] = self.location.file
        return result


@dataclass
class FieldInfo:
    name: str
    location: Location
    field_type: str | None = None
    class_name: str | None = None

    def to_dict(self, include_file: bool = True) -> dict:
        result = {
            "name": self.name,
            "line": self.location.start_line,
        }
        if include_file:
            result["file"] = self.location.file
        if self.field_type:
            result["type"] = self.field_type
        if self.class_name:
            result["class_name"] = self.class_name
        return result


class CodeAnalyzer:
    """Analyzes source code using tree-sitter."""

    def __init__(self, file_path: str | None = None, language: str | None = None):
        self.file_path = file_path
        self._language = language
        self._source: bytes | None = None
        self._tree: tree_sitter.Tree | None = None
        self._parser: tree_sitter.Parser | None = None
        self._functions_cache: list[FunctionInfo] | None = None
        self._calls_cache: list[CallInfo] | None = None
        self._classes_cache: list[ClassInfo] | None = None
        self._imports_cache: list[ImportInfo] | None = None
        self._variables_cache: list[VariableInfo] | None = None
        self._strings_cache: list[StringLiteral] | None = None
        self._fields_cache: dict[str, list[FieldInfo]] = {}

        if file_path:
            self._load_file(file_path)

    def _load_file(self, file_path: str) -> None:
        """Load file content and detect language (lazy parsing)."""
        path = Path(file_path)
        if not path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")

        try:
            self._source = path.read_bytes()
        except OSError as e:
            raise ValueError(f"Could not read file {file_path}: {e}")

        if not self._language:
            self._language = detect_language(file_path)

        if not self._language:
            raise ValueError(f"Could not detect language for: {file_path}")

        self._parser = get_parser(self._language)
        if not self._parser:
            raise ValueError(f"Unsupported language: {self._language}")

    def _ensure_tree(self) -> None:
        """Parse the source code if not already parsed (lazy parsing)."""
        if self._tree is not None:
            return
        if self._parser is None or self._source is None:
            return
        self._tree = self._parser.parse(self._source)

    def _node_location(self, node: tree_sitter.Node) -> Location:
        return Location(
            file=self.file_path or "<string>",
            start_line=node.start_point.row + 1,
            end_line=node.end_point.row + 1,
        )

    def _node_text(self, node: tree_sitter.Node) -> str:
        if self._source is None:
            return ""
        return self._source[node.start_byte : node.end_byte].decode("utf-8", errors="replace")

    def _run_query(self, query_str: str) -> dict[str, list[tree_sitter.Node]]:
        self._ensure_tree()
        if not self._tree or not self._language:
            return {}

        query = _get_compiled_query(self._language, query_str)
        if not query:
            return {}

        try:
            cursor = tree_sitter.QueryCursor(query)
            return cursor.captures(self._tree.root_node)
        except Exception:
            return {}

    def _run_query_matches(self, query_str: str) -> list[dict[str, tree_sitter.Node]]:
        """Run a query and return a list of matches, preserving capture relationships."""
        self._ensure_tree()
        if not self._tree or not self._language:
            return []

        query = _get_compiled_query(self._language, query_str)
        if not query:
            return []

        try:
            cursor = tree_sitter.QueryCursor(query)
            matches = cursor.matches(self._tree.root_node)
            
            results = []
            for match in matches:
                # Handle different tree-sitter versions/bindings
                if isinstance(match, tuple) and len(match) == 2:
                    _, captures = match
                elif isinstance(match, tuple) and len(match) > 2:
                     # Maybe (match_id, pattern_index, captures)
                     captures = match[-1]
                else:
                    captures = match

                match_dict = {}
                if isinstance(captures, dict):
                     for name, node in captures.items():
                         if isinstance(node, list):
                             match_dict[name] = node[0]
                         else:
                             match_dict[name] = node
                elif isinstance(captures, list):
                    for item in captures:
                        # item is (Node, str) or (Node, str, ...)
                        if isinstance(item, tuple) and len(item) >= 2:
                             # usually (node, name)
                             node = item[0]
                             name = item[1]
                             match_dict[name] = node
                
                results.append(match_dict)
            return results
        except Exception as e:
            print(f"Error running query matches: {e}")
            return []

    def _find_enclosing_function(self, node: tree_sitter.Node) -> str | None:
        current = node.parent
        func_types = {
            "function_definition",
            "async_function_definition",
            "function_declaration",
            "method_definition",
            "arrow_function",
            "method_declaration",
            "constructor_declaration",
            "function_expression",
            "func_literal",
        }
        anonymous_types = {"arrow_function", "func_literal"}
        while current:
            if current.type in func_types:
                name_node = current.child_by_field_name("name")
                if name_node:
                    return self._node_text(name_node)
                if current.type in anonymous_types:
                    inferred = self._infer_anonymous_function_name(current)
                    return inferred
                if current.type == "function_expression":
                    inferred = self._infer_anonymous_function_name(current)
                    return inferred
                for child in current.children:
                    if child.type in ("identifier", "property_identifier", "field_identifier"):
                        return self._node_text(child)
            current = current.parent
        return None

    def _infer_anonymous_function_name(self, func_node: tree_sitter.Node) -> str | None:
        """Infer the name of an anonymous function from its assignment context."""
        parent = func_node.parent
        if not parent:
            return None
        if parent.type == "variable_declarator":
            name_node = parent.child_by_field_name("name")
            if name_node and name_node.type == "identifier":
                return self._node_text(name_node)
        elif parent.type == "assignment_expression":
            left_node = parent.child_by_field_name("left")
            if left_node and left_node.type == "identifier":
                return self._node_text(left_node)
        elif parent.type in ("pair", "property"):
            key_node = parent.child_by_field_name("key")
            if key_node and key_node.type in ("identifier", "property_identifier", "string"):
                text = self._node_text(key_node)
                return text.strip("\"'")
        elif parent.type == "assignment":
            left_node = parent.child_by_field_name("left")
            if left_node and left_node.type == "identifier":
                return self._node_text(left_node)
        return None

    def _find_enclosing_class(self, node: tree_sitter.Node) -> str | None:
        """Find the name of the class that encloses this node."""
        class_types = {
            "class_definition",
            "class_declaration",
            "class_body",
            "interface_declaration",
        }
        
        # Go: method_declaration itself contains the receiver (class) info
        if self._language == "go" and node.type == "method_declaration":
             receiver_class = self._extract_go_receiver_type(node)
             if receiver_class:
                 return receiver_class

        current = node.parent
        while current:
            if self._language == "go" and current.type == "method_declaration":
                receiver_class = self._extract_go_receiver_type(current)
                if receiver_class:
                    return receiver_class
            if current.type in class_types:
                for child in current.children:
                    if child.type in ("identifier", "type_identifier", "name"):
                        return self._node_text(child)
            current = current.parent
        return None

    def _unwrap_go_type(self, type_node: tree_sitter.Node) -> str | None:
        """Unwrap Go type (pointer, generic) to get the base type name."""
        # print(f"Unwrapping type: {type_node.type} {self._node_text(type_node)}")
        if type_node.type == "type_identifier":
            return self._node_text(type_node)
        elif type_node.type == "pointer_type":
            # *Type or *Generic[T]
            for child in type_node.children:
                if child.type != "*":
                    return self._unwrap_go_type(child)
        elif type_node.type == "generic_type":
            # Generic[T] -> Generic
            base_type = type_node.child_by_field_name("type")
            if base_type:
                return self._unwrap_go_type(base_type)
            # Fallback
            for child in type_node.children:
                if child.type == "type_identifier":
                    return self._node_text(child)
        return None

    def _extract_go_receiver_type(self, method_node: tree_sitter.Node) -> str | None:
        """Extract receiver type from a Go method_declaration node."""
        # Try to get receiver field first
        receiver_list = method_node.child_by_field_name("receiver")
        if not receiver_list:
            # Fallback: search for first parameter_list
            for child in method_node.children:
                if child.type == "parameter_list":
                    receiver_list = child
                    break
        
        if not receiver_list:
            return None

        for param in receiver_list.children:
            if param.type == "parameter_declaration":
                type_node = param.child_by_field_name("type")
                if type_node:
                    return self._unwrap_go_type(type_node)
                
                # Fallback to children traversal if type field is missing (rare)
                for p in param.children:
                    if p.type == "pointer_type":
                        for pt in p.children:
                            if pt.type == "type_identifier":
                                return self._node_text(pt)
                    if p.type == "type_identifier":
                        return self._node_text(p)
        return None

    def _parse_attribute_node(self, node: tree_sitter.Node) -> tuple[str, str | None]:
        """Parse an attribute/member_expression/selector_expression node.

        Returns (callee, object_name) tuple.
        For 'os.path.join', returns ('join', 'os.path').
        For 'self.method', returns ('method', 'self').
        For 'this.method', returns ('method', 'this').
        For 'os.makedirs', returns ('makedirs', 'os').
        """
        callee = ""
        obj_name = None

        if node.type == "attribute":
            obj_node = node.child_by_field_name("object")
            attr_node = node.child_by_field_name("attribute")
            if attr_node:
                callee = self._node_text(attr_node)
            if obj_node:
                obj_name = self._node_text(obj_node)
        elif node.type == "member_expression":
            obj_node = node.child_by_field_name("object")
            prop_node = node.child_by_field_name("property")
            if prop_node:
                callee = self._node_text(prop_node)
            if obj_node:
                obj_name = self._node_text(obj_node)
        elif node.type == "selector_expression":
            operand_node = node.child_by_field_name("operand")
            field_node = node.child_by_field_name("field")
            if field_node:
                callee = self._node_text(field_node)
            if operand_node:
                obj_name = self._node_text(operand_node)
        else:
            id_types = (
                "identifier",
                "property_identifier",
                "private_property_identifier",
                "field_identifier",
            )
            attr_types = ("attribute", "member_expression", "selector_expression")
            ids = []
            for child in node.children:
                if child.type in id_types:
                    ids.append(self._node_text(child))
                elif child.type in attr_types:
                    obj_name = self._node_text(child)
            if ids:
                callee = ids[-1]
                if len(ids) > 1 and obj_name is None:
                    obj_name = ids[0]

        return callee, obj_name

    def get_functions(self) -> list[FunctionInfo]:
        if self._functions_cache is not None:
            return self._functions_cache

        if not self._language:
            return []

        lang_info = get_language_info(self._language)
        if not lang_info:
            return []

        matches = self._run_query_matches(lang_info.function_query)
        
        # Extract function/name pairs
        func_pairs = []
        for match in matches:
            func_node = match.get("function")
            name_node = match.get("name")
            if func_node and name_node:
                func_pairs.append((func_node, name_node))
        
        # Sort nodes by start_byte to ensure outer functions are processed first
        func_pairs.sort(key=lambda p: (p[0].start_byte, -p[0].end_byte))

        functions = []
        func_ranges: list[tuple[int, int]] = []
        
        for func_node, name_node in func_pairs:
            name = self._node_text(name_node)
            if not name:
                continue

            start, end = func_node.start_byte, func_node.end_byte
            is_nested = any(s < start and end <= e for s, e in func_ranges)
            if is_nested:
                continue
            
            func_ranges.append((start, end))
            class_name = self._find_enclosing_class(func_node)
            functions.append(
                FunctionInfo(
                    name=name,
                    location=self._node_location(func_node),
                    body=self._node_text(func_node),
                    node=func_node,
                    is_method=class_name is not None,
                    class_name=class_name,
                )
            )

        self._functions_cache = functions
        return functions

    def get_function_by_name(self, name: str, class_name: str | None = None) -> FunctionInfo | None:
        """Get a specific function by name, optionally filtering by class_name."""
        for func in self.get_functions():
            if func.name == name and (class_name is None or func.class_name == class_name):
                return func
        return None

    def get_all_functions_by_name(
        self, name: str, class_name: str | None = None
    ) -> list[FunctionInfo]:
        """Get all functions with a given name, optionally filtering by class_name."""
        results = []
        for func in self.get_functions():
            if func.name == name and (class_name is None or func.class_name == class_name):
                results.append(func)
        return results

    def get_function_variables(
        self, function_name: str, class_name: str | None = None
    ) -> list[VariableInfo]:
        """Get all variables declared within a specific function."""
        funcs = self.get_all_functions_by_name(function_name, class_name)
        all_vars = self.get_variables()
        results = []
        for func in funcs:
            results.extend(
                v
                for v in all_vars
                if func.location.start_line <= v.location.start_line <= func.location.end_line
            )
        return results

    def get_function_strings(
        self, function_name: str, class_name: str | None = None
    ) -> list[StringLiteral]:
        """Get all string literals within a specific function."""
        funcs = self.get_all_functions_by_name(function_name, class_name)
        all_strings = self.get_strings()
        results = []
        for func in funcs:
            if func.node:
                results.extend(
                    s
                    for s in all_strings
                    if func.location.start_line <= s.location.start_line <= func.location.end_line
                )
        return results

    def get_function_body(self, function_name: str) -> str | None:
        """Get the source code body of a specific function."""
        func = self.get_function_by_name(function_name)
        if func:
            return func.body
        return None

    def get_function_callees(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Get all functions/methods called by a specific function.

        Works for both named functions (from get_functions()) and inferred names
        from anonymous functions like arrow functions.
        """
        funcs = self.get_all_functions_by_name(function_name, class_name)
        all_calls = self.get_calls()
        callees: list[dict] = []
        seen: set[tuple[str, str | None]] = set()

        if funcs:
            for func in funcs:
                for call in all_calls:
                    if call.caller == func.name and call.caller_class_name == func.class_name:
                        callee = call.callee
                        if call.object_name:
                            callee = f"{call.object_name}.{callee}"
                        key = (callee, func.class_name)
                        if key not in seen:
                            seen.add(key)
                            callees.append(
                                {
                                    "callee": callee,
                                    "line": call.location.start_line,
                                    "class_name": func.class_name,
                                }
                            )
        else:
            for call in all_calls:
                if call.caller == function_name:
                    if class_name is not None and call.caller_class_name != class_name:
                        continue
                    callee = call.callee
                    if call.object_name:
                        callee = f"{call.object_name}.{callee}"
                    key = (callee, call.caller_class_name)
                    if key not in seen:
                        seen.add(key)
                        callees.append(
                            {
                                "callee": callee,
                                "line": call.location.start_line,
                                "class_name": call.caller_class_name,
                            }
                        )
        return callees

    def get_function_callers(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Get all functions that call a specific function.

        Note: class_name is used to filter calls by the object name (e.g., self.method()).
        """
        all_calls = self.get_calls()
        callers: list[dict] = []
        seen: set[tuple[str, int]] = set()
        for call in all_calls:
            if call.callee != function_name:
                continue
            if class_name is not None:
                matches_explicit_target = call.object_name == class_name
                matches_implicit_same_class = (
                    call.object_name is None and call.caller_class_name == class_name
                )
                matches_this_qualifier = (
                    call.object_name == "this" and call.caller_class_name == class_name
                )
                matches_self_qualifier = (
                    call.object_name == "self" and call.caller_class_name == class_name
                )
                if not (
                    matches_explicit_target
                    or matches_implicit_same_class
                    or matches_this_qualifier
                    or matches_self_qualifier
                ):
                    continue
            caller = call.caller or "<module>"
            key = (caller, call.location.start_line)
            if key not in seen:
                seen.add(key)
                callers.append(
                    {
                        "caller": caller,
                        "line": call.location.start_line,
                        "target_class": class_name,
                    }
                )
        return callers

    def get_classes(self) -> list[ClassInfo]:
        if self._classes_cache is not None:
            return self._classes_cache

        if not self._language:
            return []

        lang_info = get_language_info(self._language)
        if not lang_info:
            return []

        methods_by_class: dict[str, set[str]] = {}
        if self._language == "go":
            for func in self.get_functions():
                if func.class_name:
                    methods_by_class.setdefault(func.class_name, set()).add(func.name)

        matches = self._run_query_matches(lang_info.class_query)
        
        class_pairs = []
        for match in matches:
            class_node = match.get("class")
            name_node = match.get("name")
            if class_node and name_node:
                class_pairs.append((class_node, name_node))
                
        # Sort nodes by start_byte to ensure outer classes are processed first
        class_pairs.sort(key=lambda p: (p[0].start_byte, -p[0].end_byte))

        classes = []
        class_ranges: list[tuple[int, int]] = []
        
        for class_node, name_node in class_pairs:
            name = self._node_text(name_node)
            if not name:
                continue

            start, end = class_node.start_byte, class_node.end_byte
            is_nested = any(s < start and end <= e for s, e in class_ranges)
            if is_nested:
                continue
            
            class_ranges.append((start, end))
            methods = self._extract_methods_from_class(class_node)
            if self._language == "go":
                methods = sorted(set(methods) | methods_by_class.get(name, set()))
            fields = self._extract_fields_from_class(class_node)
            super_classes = self._extract_super_classes_from_class(class_node)
            classes.append(
                ClassInfo(
                    name=name,
                    location=self._node_location(class_node),
                    methods=methods,
                    fields=fields,
                    super_classes=super_classes,
                )
            )

        self._classes_cache = classes
        return classes

    def _extract_methods_from_class(self, class_node: tree_sitter.Node) -> list[str]:
        """Extract method names from a class node."""
        methods = []
        method_types = {
            "function_definition",
            "method_definition",
            "method_declaration",
            "constructor_declaration",
            "method_elem",
            "method_spec",
        }

        def walk(node: tree_sitter.Node):
            if node.type in method_types:
                for child in node.children:
                    if child.type in ("identifier", "property_identifier", "field_identifier"):
                        methods.append(self._node_text(child))
                        break
                    if child.type == "name":
                        methods.append(self._node_text(child))
                        break
                return
            for child in node.children:
                walk(child)

        walk(class_node)
        return methods

    def _get_class_body_node(self, class_node: tree_sitter.Node) -> tree_sitter.Node | None:
        body = class_node.child_by_field_name("body")
        if body is not None:
            return body
        for child in class_node.children:
            if child.type == "class_body":
                return child
        return None

    def _clean_type_text(self, text: str) -> str:
        stripped = text.strip()
        if stripped.startswith(":"):
            return stripped[1:].strip()
        return stripped

    def _extract_field_infos_js_like(
        self, class_node: tree_sitter.Node, class_name: str
    ) -> list[FieldInfo]:
        body = self._get_class_body_node(class_node)
        if body is None:
            return []

        fields: list[FieldInfo] = []
        seen: set[str] = set()

        def add_field(name: str, node: tree_sitter.Node, field_type: str | None):
            if not name or name in seen:
                return
            seen.add(name)
            fields.append(
                FieldInfo(
                    name=name,
                    location=self._node_location(node),
                    field_type=field_type,
                    class_name=class_name,
                )
            )

        for member in (c for c in body.children if c.is_named):
            if member.type.endswith("field_definition") or member.type in {
                "property_definition",
                "field_definition",
            }:
                name_node = (
                    member.child_by_field_name("name")
                    or member.child_by_field_name("property")
                    or member.child_by_field_name("pattern")
                )
                if name_node is None:
                    continue

                if name_node.type in {
                    "identifier",
                    "property_identifier",
                    "private_property_identifier",
                    "field_identifier",
                }:
                    name = self._node_text(name_node)
                else:
                    continue

                type_node = member.child_by_field_name("type")
                field_type = None
                if type_node is not None:
                    field_type = self._clean_type_text(self._node_text(type_node))

                add_field(name, member, field_type)

            if member.type == "method_definition":
                name_node = member.child_by_field_name("name")
                if name_node is None or self._node_text(name_node) != "constructor":
                    continue

                params = member.child_by_field_name("parameters")
                if params is None:
                    continue

                for param in (c for c in params.children if c.is_named):
                    has_param_property_modifier = any(
                        c.type in {"accessibility_modifier", "readonly"} for c in param.children
                    )
                    if not has_param_property_modifier:
                        continue

                    pattern = param.child_by_field_name("pattern") or param.child_by_field_name(
                        "name"
                    )
                    if pattern is None:
                        continue

                    if pattern.type == "assignment_pattern":
                        pattern = pattern.child_by_field_name("left") or pattern

                    if pattern.type not in {"identifier", "property_identifier"}:
                        continue

                    param_name = self._node_text(pattern)

                    type_node = param.child_by_field_name("type")
                    param_type = None
                    if type_node is not None:
                        param_type = self._clean_type_text(self._node_text(type_node))

                    add_field(param_name, param, param_type)

                # Scan constructor body for assignments to this.prop
                body_node = member.child_by_field_name("body")
                if body_node:
                    self._extract_fields_from_constructor_body(body_node, add_field)

        return fields

    def _extract_fields_from_constructor_body(
        self, body_node: tree_sitter.Node, add_field_callback: Callable
    ) -> None:
        """Scan constructor body for 'this.prop = value' assignments."""
        def walk(node: tree_sitter.Node):
            if node.type == "assignment_expression":
                left = node.child_by_field_name("left")
                if left and left.type == "member_expression":
                    obj = left.child_by_field_name("object")
                    prop = left.child_by_field_name("property")
                    if obj and self._node_text(obj) == "this" and prop:
                        name = self._node_text(prop)
                        add_field_callback(name, node, None)
            
            # Recurse but stop at function boundaries to avoid capturing nested function assignments
            if node.type in {
                "function_declaration", "function_expression", "arrow_function", 
                "method_definition", "class_declaration", "class_expression"
            } and node != body_node:
                return

            for child in node.children:
                walk(child)

        walk(body_node)

    def _extract_fields_from_class(self, class_node: tree_sitter.Node) -> list[str]:
        """Extract field names from a class node."""
        class_name_node = class_node.child_by_field_name("name")
        class_name = self._node_text(class_name_node) if class_name_node else ""
        return [f.name for f in self._extract_field_infos(class_node, class_name)]

    def _extract_super_classes_from_class(self, class_node: tree_sitter.Node) -> list[str]:
        """Extract parent class names from a class node."""
        super_classes: list[str] = []

        if self._language == "python":
            for child in class_node.children:
                if child.type == "argument_list":
                    for arg in child.children:
                        if arg.type == "identifier" or arg.type == "attribute":
                            super_classes.append(self._node_text(arg))

        elif self._language in {"javascript", "typescript", "tsx"}:
            seen: set[str] = set()

            def add_name(name: str) -> None:
                name = name.strip()
                if not name or name in seen:
                    return
                seen.add(name)
                super_classes.append(name)

            def handle_expression(node: tree_sitter.Node | None) -> None:
                if node is None:
                    return

                if node.type in {"identifier", "type_identifier", "property_identifier"}:
                    add_name(self._node_text(node))
                    return

                if node.type == "member_expression":
                    add_name(self._node_text(node))
                    for c in reversed(node.children):
                        if c.type in {"identifier", "property_identifier"}:
                            add_name(self._node_text(c))
                            break
                    return

                if node.type == "expression_with_type_arguments":
                    expr = node.child_by_field_name("expression")
                    if expr is not None:
                        handle_expression(expr)
                        return
                    for c in node.named_children:
                        handle_expression(c)
                        return
                    return
                
                if node.type == "call_expression":
                     func = node.child_by_field_name("function")
                     if func:
                         handle_expression(func)
                     return

                # Handle wrapper nodes
                for child in node.named_children:
                     handle_expression(child)

            def walk_heritage(node: tree_sitter.Node) -> None:
                if node.type == "class_heritage":
                    for child in node.named_children:
                        if child.type in {"extends_clause", "implements_clause"}:
                             for grandchild in child.named_children:
                                 handle_expression(grandchild)
                        else:
                             handle_expression(child)
                    return

                if node.type in {"extends_clause", "implements_clause"}:
                    for child in node.named_children:
                         handle_expression(child)
                    return

                for child in node.children:
                    walk_heritage(child)

            for child in class_node.children:
                if child.type == "class_heritage":
                    walk_heritage(child)

        elif self._language == "java":
            for child in class_node.children:
                if child.type == "superclass":
                    for sub in child.children:
                        if sub.type == "type_identifier":
                            super_classes.append(self._node_text(sub))
                        elif sub.type == "generic_type":
                            for g in sub.children:
                                if g.type == "type_identifier":
                                    super_classes.append(self._node_text(g))
                                    break
                elif child.type == "super_interfaces":
                    for sub in child.children:
                        if sub.type == "type_list":
                            for t in sub.children:
                                if t.type == "type_identifier":
                                    super_classes.append(self._node_text(t))
                                elif t.type == "generic_type":
                                    for g in t.children:
                                        if g.type == "type_identifier":
                                            super_classes.append(self._node_text(g))
                                            break

        elif self._language == "go":

            def parse_embedded_type_name(node: tree_sitter.Node) -> str | None:
                if node.type == "type_identifier":
                    return self._node_text(node)

                if node.type == "qualified_type":
                    for c in node.children:
                        if c.type == "type_identifier":
                            return self._node_text(c)
                    return None

                if node.type == "generic_type":
                    for c in node.children:
                        if c.type in {"type_identifier", "qualified_type", "pointer_type"}:
                            name = parse_embedded_type_name(c)
                            if name:
                                return name
                    return None

                if node.type == "pointer_type":
                    for c in node.children:
                        if c.type in {
                            "type_identifier",
                            "qualified_type",
                            "generic_type",
                            "parenthesized_type",
                        }:
                            name = parse_embedded_type_name(c)
                            if name:
                                return name
                    return None

                if node.type == "parenthesized_type":
                    for c in node.named_children:
                        name = parse_embedded_type_name(c)
                        if name:
                            return name
                    return None

                return None

            def embedded_from_field_declaration(fd: tree_sitter.Node) -> str | None:
                if any(c.type == "field_identifier" for c in fd.children):
                    return None

                for c in fd.children:
                    if c.type == "*":
                        continue
                    if c.type in {
                        "type_identifier",
                        "qualified_type",
                        "generic_type",
                        "pointer_type",
                        "parenthesized_type",
                    }:
                        name = parse_embedded_type_name(c)
                        if name:
                            return name

                for c in fd.named_children:
                    name = parse_embedded_type_name(c)
                    if name:
                        return name

                return None

            for child in class_node.children:
                if child.type == "type_spec":
                    for sub in child.children:
                        if sub.type == "struct_type":
                            for field in sub.children:
                                if field.type == "field_declaration_list":
                                    for fd in field.children:
                                        if fd.type == "field_declaration":
                                            embedded = embedded_from_field_declaration(fd)
                                            if embedded:
                                                super_classes.append(embedded)

        return super_classes

    def get_fields(self, class_name: str) -> list[FieldInfo]:
        """Get all fields, optionally filtered by class name."""
        if class_name in self._fields_cache:
            return self._fields_cache[class_name]

        if not self._language:
            return []

        lang_info = get_language_info(self._language)
        if not lang_info:
            return []

        classes = self.get_classes()
        fields: list[FieldInfo] = []

        for cls in classes:
            if cls.name != class_name:
                continue
            field_infos = self._get_fields_from_class_node(cls.name)
            fields.extend(field_infos)

        self._fields_cache[class_name] = fields
        return fields

    def _get_fields_from_class_node(self, class_name: str) -> list[FieldInfo]:
        """Get detailed field info for a specific class."""
        if not self._language or not self._tree:
            return []

        lang_info = get_language_info(self._language)
        if not lang_info:
            return []

        captures = self._run_query(lang_info.class_query)
        class_nodes = captures.get("class", [])
        name_nodes = captures.get("name", [])

        target_class_node = None
        for class_node in class_nodes:
            for name_node in name_nodes:
                if (
                    class_node.start_byte <= name_node.start_byte
                    and name_node.end_byte <= class_node.end_byte
                    and self._node_text(name_node) == class_name
                ):
                    target_class_node = class_node
                    break
            if target_class_node:
                break

        if not target_class_node:
            return []

        return self._extract_field_infos(target_class_node, class_name)

    def _extract_field_infos(
        self, class_node: tree_sitter.Node, class_name: str
    ) -> list[FieldInfo]:
        """Extract detailed field information from a class node."""
        if self._language in {"javascript", "typescript", "tsx"}:
            return self._extract_field_infos_js_like(class_node, class_name)

        fields: list[FieldInfo] = []
        seen: set[str] = set()
        field_types = {"field_definition", "field_declaration"}
        method_types = {
            "function_definition",
            "method_definition",
            "method_declaration",
            "constructor_declaration",
        }

        def add_field(name: str, location: Location, field_type: str | None) -> None:
            if name and name not in seen:
                seen.add(name)
                fields.append(
                    FieldInfo(
                        name=name,
                        location=location,
                        field_type=field_type,
                        class_name=class_name,
                    )
                )

        def walk(node: tree_sitter.Node, inside_method: bool = False):
            if node.type in method_types:
                if self._language != "python":
                    return
                for child in node.children:
                    walk(child, inside_method=True)
                return
            if node.type in field_types:
                names = []
                field_type = None
                
                # Go: try to get type directly
                type_node = node.child_by_field_name("type")
                if type_node:
                    field_type = self._node_text(type_node)

                for child in node.children:
                    if child.type in ("identifier", "property_identifier", "field_identifier"):
                        names.append(self._node_text(child))
                    elif child.type == "variable_declarator":
                        for sub in child.children:
                            if sub.type == "identifier":
                                names.append(self._node_text(sub))
                                break
                    elif child.type in (
                        "type_annotation",
                        "type",
                        "type_identifier",
                        "integral_type",
                        "floating_point_type",
                        "boolean_type",
                        "generic_type",
                        "array_type",
                        "scoped_type_identifier",
                    ):
                        if not field_type:
                            field_type = self._node_text(child)
                
                if self._language == "go" and not names and field_type:
                    # Embedded field: name is implicit from type
                    # *T -> T, pkg.T -> T, *pkg.T -> T
                    # field_type string might be "*time.Timer" or "Reader"
                    type_str = field_type
                    if type_str.startswith("*"):
                        type_str = type_str[1:]
                    
                    if "." in type_str:
                        names.append(type_str.split(".")[-1])
                    else:
                        names.append(type_str)
                
                for name in names:
                    add_field(name, self._node_location(node), field_type)
                return
            if self._language == "python" and node.type == "expression_statement":
                for child in node.children:
                    if child.type == "assignment":
                        left_node = child.child_by_field_name("left")
                        if left_node is None:
                            continue

                        name = ""
                        field_type = None
                        
                        # Extract type if present (for annotated assignments)
                        type_node = child.child_by_field_name("type")
                        if type_node:
                            field_type = self._node_text(type_node)

                        if inside_method:
                            # Instance attributes: self.x = ...
                            if left_node.type == "attribute":
                                obj_node = left_node.child_by_field_name("object")
                                attr_node = left_node.child_by_field_name("attribute")
                                if (
                                    obj_node is not None
                                    and attr_node is not None
                                    and self._node_text(obj_node) == "self"
                                ):
                                    name = self._node_text(attr_node)
                        else:
                            # Class attributes: x = ... or x: int = ...
                            if left_node.type == "identifier":
                                name = self._node_text(left_node)

                        if name:
                            add_field(name, self._node_location(child), field_type)
                return
            for child in node.children:
                walk(child, inside_method)

        walk(class_node)
        return fields

    def get_calls(self) -> list[CallInfo]:
        if self._calls_cache is not None:
            return self._calls_cache

        if not self._language:
            return []

        lang_info = get_language_info(self._language)
        if not lang_info:
            return []

        captures = self._run_query(lang_info.call_query)
        call_nodes = captures.get("call", [])

        calls = []
        for call_node in call_nodes:
            caller = self._find_enclosing_function(call_node)
            caller_class_name = self._find_enclosing_class(call_node)
            callee = ""
            is_method = False
            obj_name = None

            if self._language == "java":
                if call_node.type == "explicit_constructor_invocation":
                    ctor_node = call_node.child_by_field_name("constructor")
                    if ctor_node:
                        callee = self._node_text(ctor_node)
                elif call_node.type == "object_creation_expression":
                    type_node = call_node.child_by_field_name("type")
                    if type_node:
                        if type_node.type == "generic_type":
                            for child in type_node.children:
                                if child.type == "type_identifier":
                                    callee = self._node_text(child)
                                    break
                        else:
                            callee = self._node_text(type_node)
                else:
                    name_node = call_node.child_by_field_name("name")
                    object_node = call_node.child_by_field_name("object")
                    if name_node:
                        callee = self._node_text(name_node)
                    if object_node:
                        is_method = True
                        obj_name = self._node_text(object_node)
            else:
                func_node = call_node.child_by_field_name("function")
                if func_node:
                    if func_node.type == "identifier":
                        callee = self._node_text(func_node)
                    elif func_node.type in (
                        "attribute",
                        "member_expression",
                        "selector_expression",
                    ):
                        is_method = True
                        callee, obj_name = self._parse_attribute_node(func_node)

            if callee:
                calls.append(
                    CallInfo(
                        callee=callee,
                        location=self._node_location(call_node),
                        caller=caller,
                        caller_class_name=caller_class_name,
                        is_method_call=is_method,
                        object_name=obj_name,
                    )
                )

        self._calls_cache = calls
        return calls

    def get_imports(self) -> list[ImportInfo]:
        if self._imports_cache is not None:
            return self._imports_cache

        if not self._language:
            return []

        lang_info = get_language_info(self._language)
        if not lang_info:
            return []

        captures = self._run_query(lang_info.import_query)
        module_nodes = captures.get("module", [])
        import_nodes = captures.get("import", [])

        imports = []
        for node in module_nodes:
            text = self._node_text(node).strip("\"'")
            imports.append(ImportInfo(module=text, location=self._node_location(node)))

        if not module_nodes:
            for node in import_nodes:
                text = self._node_text(node)
                imports.append(ImportInfo(module=text, location=self._node_location(node)))

        self._imports_cache = imports
        return imports

    def get_variables(self) -> list[VariableInfo]:
        if self._variables_cache is not None:
            return self._variables_cache

        if not self._language:
            return []

        lang_info = get_language_info(self._language)
        if not lang_info:
            return []

        captures = self._run_query(lang_info.variable_query)
        name_nodes = captures.get("name", [])

        variables = []
        for node in name_nodes:
            scope = self._find_enclosing_function(node)
            variables.append(
                VariableInfo(
                    name=self._node_text(node),
                    location=self._node_location(node),
                    scope=scope,
                )
            )

        self._variables_cache = variables
        return variables

    def get_strings(self) -> list[StringLiteral]:
        if self._strings_cache is not None:
            return self._strings_cache

        if not self._language:
            return []

        lang_info = get_language_info(self._language)
        if not lang_info:
            return []

        captures = self._run_query(lang_info.string_query)
        string_nodes = captures.get("string", [])

        strings = []
        for node in string_nodes:
            text = self._node_text(node)
            strings.append(StringLiteral(value=text, location=self._node_location(node)))

        self._strings_cache = strings
        return strings

    def find_symbols(self, name: str) -> list[dict]:
        if not self._source:
            return []

        name_bytes = name.encode("utf-8")
        if name_bytes not in self._source:
            return []

        self._ensure_tree()
        if not self._tree:
            return []

        refs: list[dict] = []
        cursor = self._tree.walk()
        visited_children = False

        while True:
            if not visited_children:
                node = cursor.node
                if node.is_named and self._node_text(node) == name:
                    refs.append(
                        {
                            "type": node.type,
                            "location": self._node_location(node).to_dict(),
                            "context": self._node_text(node.parent) if node.parent else "",
                        }
                    )
                
                if cursor.goto_first_child():
                    continue

            if cursor.goto_next_sibling():
                visited_children = False
            elif cursor.goto_parent():
                visited_children = True
            else:
                break

        return refs

    def get_class_by_name(self, class_name: str) -> ClassInfo | None:
        """Get a specific class by name."""
        for cls in self.get_classes():
            if cls.name == class_name:
                return cls
        return None

    def get_super_classes(self, class_name: str) -> list[ClassInfo]:
        """Get all parent classes (ancestors) of a specific class using BFS.

        Returns ClassInfo for each parent class that can be found in this file.
        """
        all_classes = self.get_classes()
        class_map = {c.name: c for c in all_classes}

        if class_name not in class_map:
            return []

        target_class = class_map[class_name]

        result = []
        visited = {class_name}
        queue = deque([target_class])

        while queue:
            current_class = queue.popleft()

            for parent_name in current_class.super_classes:
                if parent_name in class_map and parent_name not in visited:
                    parent_class = class_map[parent_name]
                    visited.add(parent_name)
                    result.append(parent_class)
                    queue.append(parent_class)

        return result

    def get_sub_classes(self, class_name: str) -> list[ClassInfo]:
        """Get all child classes (descendants) that inherit from a specific class using BFS.

        Returns ClassInfo for each child class found in this file.
        """
        all_classes = self.get_classes()

        inheritance_map = {}
        for cls in all_classes:
            for parent in cls.super_classes:
                if parent not in inheritance_map:
                    inheritance_map[parent] = []
                inheritance_map[parent].append(cls)

        result = []
        visited = {class_name}
        queue = deque([class_name])

        while queue:
            current_name = queue.popleft()

            if current_name in inheritance_map:
                children = inheritance_map[current_name]
                for child in children:
                    if child.name not in visited:
                        visited.add(child.name)
                        result.append(child)
                        queue.append(child.name)

        return result
