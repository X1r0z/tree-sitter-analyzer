"""Project-level code analysis with directory support."""

from __future__ import annotations

from collections import OrderedDict, deque

from .analyzer import CodeAnalyzer
from .nodes import (
    CallInfo,
    ClassInfo,
    FieldInfo,
    FunctionInfo,
    ImportInfo,
    VariableInfo,
)
from .parallel import run_parallel
from .utils import find_files, match_query, rg_search_files


class ProjectAnalyzer:
    """Analyzes multiple source files in a project."""

    MAX_CACHED_ANALYZERS: int = 1000
    PARALLEL_THRESHOLD: int = 10

    def __init__(self, path: str):
        """Initialize the project analyzer with a directory path."""
        self.path = path
        self.files = find_files(path)
        self._analyzers: OrderedDict[str, CodeAnalyzer] = OrderedDict()
        self._files_set: set[str] = set(self.files)

    def _filter_files_by_text(self, text: str) -> list[str]:
        """Return the subset of self.files that contain *text* using ripgrep."""
        matched = set(rg_search_files(text, self.path))
        return [f for f in self.files if f in matched]

    def _get_analyzer(self, file_path: str) -> CodeAnalyzer | None:
        """Get or create an analyzer for a file with LRU eviction."""
        if file_path in self._analyzers:
            self._analyzers.move_to_end(file_path)
            return self._analyzers[file_path]

        try:
            analyzer = CodeAnalyzer(file_path)
        except Exception:
            return None

        self._analyzers[file_path] = analyzer

        while len(self._analyzers) > self.MAX_CACHED_ANALYZERS:
            self._analyzers.popitem(last=False)

        return analyzer

    def get_functions(self, query: str = "") -> list[FunctionInfo]:
        """Get all functions from all files."""
        # Fast path optimization for simple queries
        is_simple_query = query and not any(c in query for c in ".^$*+?{}[]|()\\")
        candidate_files = self._filter_files_by_text(query) if is_simple_query else self.files

        if not candidate_files:
            return []

        def find_functions(file_path: str, q: str) -> list[FunctionInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return [f for f in analyzer.get_functions() if match_query(f.name, q)]
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            functions = []
            for file_path in candidate_files:
                functions.extend(find_functions(file_path, query))
        else:
            functions = run_parallel(candidate_files, find_functions, query)

        return functions

    def get_classes(self, query: str = "") -> list[ClassInfo]:
        """Get all classes from all files."""
        # Fast path optimization for simple queries
        is_simple_query = query and not any(c in query for c in ".^$*+?{}[]|()\\")
        candidate_files = self._filter_files_by_text(query) if is_simple_query else self.files

        if not candidate_files:
            return []

        def find_classes(file_path: str, q: str) -> list[ClassInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return [c for c in analyzer.get_classes() if match_query(c.name, q)]
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            classes = []
            for file_path in candidate_files:
                classes.extend(find_classes(file_path, query))
        else:
            classes = run_parallel(candidate_files, find_classes, query)

        return classes

    def get_fields(self, class_name: str) -> list[FieldInfo]:
        """Get all fields from all files, optionally filtered by class name."""
        candidate_files = self._filter_files_by_text(class_name)

        if not candidate_files:
            return []

        def find_fields(file_path: str, cls_name: str) -> list[FieldInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return analyzer.get_fields(cls_name)
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            fields = []
            for file_path in candidate_files:
                fields.extend(find_fields(file_path, class_name))
        else:
            fields = run_parallel(candidate_files, find_fields, class_name)

        return fields

    def get_calls(self) -> list[CallInfo]:
        """Get all function calls from all files."""
        candidate_files = self.files

        if not candidate_files:
            return []

        def find_calls(file_path: str) -> list[CallInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return analyzer.get_calls()
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            calls = []
            for file_path in candidate_files:
                calls.extend(find_calls(file_path))
        else:
            calls = run_parallel(candidate_files, find_calls)

        return calls

    def get_imports(self, query: str = "") -> list[ImportInfo]:
        """Get all imports from all files."""
        # Fast path optimization for simple queries
        is_simple_query = query and not any(c in query for c in ".^$*+?{}[]|()\\")
        candidate_files = self._filter_files_by_text(query) if is_simple_query else self.files

        if not candidate_files:
            return []

        def find_imports(file_path: str, q: str) -> list[ImportInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return [i for i in analyzer.get_imports() if match_query(i.module, q)]
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            imports = []
            for file_path in candidate_files:
                imports.extend(find_imports(file_path, query))
        else:
            imports = run_parallel(candidate_files, find_imports, query)

        return imports

    def get_variables(self, query: str = "") -> list[VariableInfo]:
        """Get all variables from all files."""
        # Fast path optimization for simple queries
        is_simple_query = query and not any(c in query for c in ".^$*+?{}[]|()\\")
        candidate_files = self._filter_files_by_text(query) if is_simple_query else self.files

        if not candidate_files:
            return []

        def find_variables(file_path: str, q: str) -> list[VariableInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return [v for v in analyzer.get_variables() if match_query(v.name, q)]
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            variables = []
            for file_path in candidate_files:
                variables.extend(find_variables(file_path, query))
        else:
            variables = run_parallel(candidate_files, find_variables, query)

        return variables

    def get_function_by_name(self, name: str, class_name: str | None = None) -> FunctionInfo | None:
        """Find a function by name across all files, optionally filtering by class_name."""
        for file_path in self._filter_files_by_text(name):
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                func = analyzer.get_function_by_name(name, class_name)
                if func:
                    return func
        return None

    def get_all_functions_by_name(
        self, name: str, class_name: str | None = None
    ) -> list[FunctionInfo]:
        """Find all functions with a given name across all files, optionally filtering by class_name."""
        candidate_files = self._filter_files_by_text(name)

        if not candidate_files:
            return []

        def find_funcs(file_path: str, fn_name: str, cls_name: str | None) -> list[FunctionInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return analyzer.get_all_functions_by_name(fn_name, cls_name)
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            functions = []
            for file_path in candidate_files:
                functions.extend(find_funcs(file_path, name, class_name))
        else:
            functions = run_parallel(candidate_files, find_funcs, name, class_name)

        return functions

    def get_callers(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Find all callers of a function across all files."""
        relevant_files = self._filter_files_by_text(function_name)

        if not relevant_files:
            return []

        def find_callers(file_path: str, fn_name: str, cls_name: str | None) -> list[dict]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return [
                    {
                        "caller": c["caller"],
                        "line": c["line"],
                        "file": file_path,
                        "target_class": c.get("target_class"),
                    }
                    for c in analyzer.get_function_callers(fn_name, cls_name)
                ]
            except Exception:
                return []

        if len(relevant_files) < self.PARALLEL_THRESHOLD:
            callers = []
            for file_path in relevant_files:
                callers.extend(find_callers(file_path, function_name, class_name))
        else:
            callers = run_parallel(relevant_files, find_callers, function_name, class_name)

        return sorted(callers, key=lambda x: (x["file"], x["line"]))

    def get_callees(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Find all functions called by a function across all files."""
        relevant_files = self._filter_files_by_text(function_name)

        if not relevant_files:
            return []

        def find_callees(file_path: str, fn_name: str, cls_name: str | None) -> list[dict]:
            try:
                analyzer = CodeAnalyzer(file_path)
                if not analyzer.get_all_functions_by_name(fn_name, cls_name):
                    return []
                return [
                    {
                        "callee": c["callee"],
                        "line": c["line"],
                        "file": file_path,
                        "class_name": c.get("class_name"),
                    }
                    for c in analyzer.get_function_callees(fn_name, cls_name)
                ]
            except Exception:
                return []

        if len(relevant_files) < self.PARALLEL_THRESHOLD:
            callees = []
            for file_path in relevant_files:
                callees.extend(find_callees(file_path, function_name, class_name))
        else:
            callees = run_parallel(relevant_files, find_callees, function_name, class_name)

        return sorted(callees, key=lambda x: (x["file"], x["line"]))

    def get_function_variables(
        self, function_name: str, class_name: str | None = None
    ) -> list[dict]:
        """Get all variables in a function across all files."""
        candidate_files = self._filter_files_by_text(function_name)

        if not candidate_files:
            return []

        def find_fn_vars(file_path: str, fn_name: str, cls_name: str | None) -> list[dict]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return [
                    {"name": v.name, "line": v.location.start_line, "file": file_path}
                    for v in analyzer.get_function_variables(fn_name, cls_name)
                ]
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            fn_variables = []
            for file_path in candidate_files:
                fn_variables.extend(find_fn_vars(file_path, function_name, class_name))
        else:
            fn_variables = run_parallel(candidate_files, find_fn_vars, function_name, class_name)

        return sorted(fn_variables, key=lambda x: (x["file"], x["line"]))

    def get_function_strings(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Get all strings in a function across all files."""
        candidate_files = self._filter_files_by_text(function_name)

        if not candidate_files:
            return []

        def find_fn_strings(file_path: str, fn_name: str, cls_name: str | None) -> list[dict]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return [
                    {"value": s.value, "line": s.location.start_line, "file": file_path}
                    for s in analyzer.get_function_strings(fn_name, cls_name)
                ]
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            fn_strings = []
            for file_path in candidate_files:
                fn_strings.extend(find_fn_strings(file_path, function_name, class_name))
        else:
            fn_strings = run_parallel(candidate_files, find_fn_strings, function_name, class_name)

        return sorted(fn_strings, key=lambda x: (x["file"], x["line"]))

    def find_symbols(self, name: str) -> list[dict]:
        """Find all references to an identifier across all files."""
        candidate_files = self._filter_files_by_text(name)

        if not candidate_files:
            return []

        def find_refs(file_path: str, symbol_name: str) -> list[dict]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return analyzer.find_symbols(symbol_name)
            except Exception:
                return []

        if len(candidate_files) < self.PARALLEL_THRESHOLD:
            refs = []
            for file_path in candidate_files:
                refs.extend(find_refs(file_path, name))
        else:
            refs = run_parallel(candidate_files, find_refs, name)

        return refs

    def get_class_by_name(self, class_name: str) -> ClassInfo | None:
        """Find a class by name across all files."""
        for file_path in self._filter_files_by_text(class_name):
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                cls = analyzer.get_class_by_name(class_name)
                if cls:
                    return cls
        return None

    def get_super_classes(self, class_name: str) -> list[ClassInfo]:
        """Get all parent classes (ancestors) of a specific class across all files using BFS.
        """
        # Find the starting class
        target_class = self.get_class_by_name(class_name)
        if not target_class:
            return []

        result = []
        visited = {class_name}
        queue = deque([target_class])

        def find_class_def(file_path: str, cls_name: str) -> list[ClassInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                cls = analyzer.get_class_by_name(cls_name)
                return [cls] if cls else []
            except Exception:
                return []

        while queue:
            current_class = queue.popleft()

            for parent_name in current_class.super_classes:
                if parent_name in visited:
                    continue
                visited.add(parent_name)

                candidate_files = self._filter_files_by_text(parent_name)
                if not candidate_files:
                    continue

                if len(candidate_files) < self.PARALLEL_THRESHOLD:
                    found = []
                    for file_path in candidate_files:
                        found.extend(find_class_def(file_path, parent_name))
                else:
                    found = run_parallel(candidate_files, find_class_def, parent_name)

                if found:
                    parent_class = found[0]
                    result.append(parent_class)
                    queue.append(parent_class)

        return result

    def get_sub_classes(self, class_name: str) -> list[ClassInfo]:
        """Get all child classes (descendants) that inherit from a specific class using BFS.
        """
        result = []
        visited = {class_name}
        queue = deque([class_name])

        def find_subclasses(file_path: str, parent_name: str) -> list[ClassInfo]:
            try:
                analyzer = CodeAnalyzer(file_path)
                return [
                    cls for cls in analyzer.get_classes()
                    if parent_name in cls.super_classes
                ]
            except Exception:
                return []

        while queue:
            current_parent_name = queue.popleft()
            candidate_files = self._filter_files_by_text(current_parent_name)

            if not candidate_files:
                continue

            if len(candidate_files) < self.PARALLEL_THRESHOLD:
                found = []
                for file_path in candidate_files:
                    found.extend(find_subclasses(file_path, current_parent_name))
            else:
                found = run_parallel(candidate_files, find_subclasses, current_parent_name)

            for cls in found:
                if cls.name not in visited:
                    visited.add(cls.name)
                    result.append(cls)
                    queue.append(cls.name)

        return result
