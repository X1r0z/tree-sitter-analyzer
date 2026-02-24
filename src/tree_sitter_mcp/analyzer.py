"""Core code analysis functionality using tree-sitter."""

from __future__ import annotations

from collections import deque

from .languages import get_language_info
from .nodes import (
    CallInfo,
    ClassInfo,
    FieldInfo,
    FunctionInfo,
    ImportInfo,
    StringLiteral,
    VariableInfo,
)
from .parser import BaseParser


class CodeAnalyzer(BaseParser):
    """Analyzes source code using tree-sitter."""

    def __init__(self, file_path: str | None = None, language: str | None = None):
        """Initialize the analyzer with a file path and language."""
        super().__init__(file_path, language)
        self._functions_cache: list[FunctionInfo] | None = None
        self._calls_cache: list[CallInfo] | None = None
        self._calls_by_callee: dict[str, list[CallInfo]] | None = None
        self._calls_by_caller: dict[str, list[CallInfo]] | None = None
        self._classes_cache: list[ClassInfo] | None = None
        self._imports_cache: list[ImportInfo] | None = None
        self._variables_cache: list[VariableInfo] | None = None
        self._strings_cache: list[StringLiteral] | None = None
        self._fields_cache: dict[str, list[FieldInfo]] = {}

    def get_functions(self) -> list[FunctionInfo]:
        """Get all functions in the file."""
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
        funcs = []
        for func in self.get_functions():
            if func.name == name and (class_name is None or func.class_name == class_name):
                funcs.append(func)
        return funcs

    def get_function_variables(
        self, function_name: str, class_name: str | None = None
    ) -> list[VariableInfo]:
        """Get all variables declared within a specific function."""
        funcs = self.get_all_functions_by_name(function_name, class_name)
        all_vars = self.get_variables()
        fn_vars = []
        for func in funcs:
            fn_vars.extend(
                v
                for v in all_vars
                if func.location.start_line <= v.location.start_line <= func.location.end_line
            )
        return fn_vars

    def get_function_strings(
        self, function_name: str, class_name: str | None = None
    ) -> list[StringLiteral]:
        """Get all string literals within a specific function."""
        funcs = self.get_all_functions_by_name(function_name, class_name)
        all_strings = self.get_strings()
        fn_strings = []
        for func in funcs:
            if func.node:
                fn_strings.extend(
                    s
                    for s in all_strings
                    if func.location.start_line <= s.location.start_line <= func.location.end_line
                )
        return fn_strings

    def get_function_body(self, function_name: str) -> str | None:
        """Get the source code body of a specific function."""
        func = self.get_function_by_name(function_name)
        if func:
            return func.body
        return None

    def get_function_callees(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Get all functions/methods called by a specific function."""
        funcs = self.get_all_functions_by_name(function_name, class_name)
        # Ensure calls are parsed and indexed
        self.get_calls()
        if self._calls_by_caller is None:
            self._calls_by_caller = {}

        callees: list[dict] = []
        seen: set[tuple[str, str | None]] = set()

        if funcs:
            for func in funcs:
                caller_calls = self._calls_by_caller.get(func.name, [])
                for call in caller_calls:
                    if call.caller_class_name == func.class_name:
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
            # Fallback if we can't find the function definition but have calls attributed to it
            caller_calls = self._calls_by_caller.get(function_name, [])
            for call in caller_calls:
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
        """Get all functions that call a specific function."""
        # Ensure calls are parsed and indexed
        self.get_calls()
        if self._calls_by_callee is None:
            self._calls_by_callee = {}

        candidate_calls = self._calls_by_callee.get(function_name, [])
        callers: list[dict] = []
        seen: set[tuple[str, int]] = set()

        for call in candidate_calls:
            if class_name is not None and not self._matches_call_target_class(call, class_name):
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
        """Get all classes in the file."""
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

    def get_calls(self) -> list[CallInfo]:
        """Get all function calls in the file."""
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

        # Build indices
        self._calls_by_callee = {}
        self._calls_by_caller = {}
        for call in calls:
            # Index by callee
            if call.callee not in self._calls_by_callee:
                self._calls_by_callee[call.callee] = []
            self._calls_by_callee[call.callee].append(call)

            # Index by caller
            if call.caller:
                if call.caller not in self._calls_by_caller:
                    self._calls_by_caller[call.caller] = []
                self._calls_by_caller[call.caller].append(call)

        return calls

    def get_imports(self) -> list[ImportInfo]:
        """Get all imports in the file."""
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
        """Get all variables in the file."""
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
        """Get all string literals in the file."""
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
        """Find all occurrences of a symbol name in the file."""
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
        """Get all parent classes (ancestors) of a specific class using BFS."""
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
        """Get all child classes (descendants) that inherit from a specific class using BFS."""
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
