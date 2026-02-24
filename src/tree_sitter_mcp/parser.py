"""Low-level AST parsing logic using tree-sitter."""

from __future__ import annotations

import re
from collections.abc import Callable
from pathlib import Path

import tree_sitter

from .languages import detect_language, get_language_info, get_parser
from .nodes import FieldInfo, Location
from .utils import get_compiled_query


class BaseParser:
    """Base class with low-level tree-sitter parsing methods."""

    def __init__(self, file_path: str | None = None, language: str | None = None):
        self.file_path = file_path
        self._language = language
        self._source: bytes | None = None
        self._tree: tree_sitter.Tree | None = None
        self._parser: tree_sitter.Parser | None = None

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
            raise ValueError(f"Could not read file {file_path}: {e}") from e

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

        query = get_compiled_query(self._language, query_str)
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

        query = get_compiled_query(self._language, query_str)
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

    def _extract_instance_attr(self, object_name: str) -> str | None:
        for prefix in ("self.", "this.", "cls."):
            if object_name.startswith(prefix):
                remainder = object_name[len(prefix) :]
                if remainder:
                    return remainder.split(".")[0]
        return None

    def _type_matches_class(self, field_type: str | None, class_name: str) -> bool:
        if not field_type:
            return False
        tokens = re.split(r"[^A-Za-z0-9_]+", field_type)
        return class_name in {t for t in tokens if t}

    def _matches_call_target_class(self, call: CallInfo, class_name: str) -> bool:
        if call.object_name == class_name:
            return True
        if call.object_name is None:
            return call.caller_class_name == class_name
        if call.object_name in {"self", "this", "cls"}:
            return call.caller_class_name == class_name
        if call.caller_class_name:
            attr_name = self._extract_instance_attr(call.object_name)
            if attr_name:
                for field in self.get_fields(call.caller_class_name):
                    if field.name != attr_name:
                        continue
                    if self._type_matches_class(field.field_type, class_name):
                        return True
        return False

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
            if (
                node.type
                in {
                    "function_declaration",
                    "function_expression",
                    "arrow_function",
                    "method_definition",
                    "class_declaration",
                    "class_expression",
                }
                and node != body_node
            ):
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
                    elif not field_type and child.type in (
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
