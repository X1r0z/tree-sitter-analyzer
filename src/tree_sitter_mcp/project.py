"""Project-level code analysis with directory support."""

from __future__ import annotations

from collections import OrderedDict, deque
from pathlib import Path

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
from .utils import find_files, match_query


class ProjectAnalyzer:
    """Analyzes multiple source files in a project."""

    MAX_CACHED_ANALYZERS: int = 1000
    PARALLEL_THRESHOLD: int = 10

    def __init__(self, path: str):
        """Initialize the project analyzer with a directory path."""
        self.path = path
        self.files = find_files(path)
        self._analyzers: OrderedDict[str, CodeAnalyzer] = OrderedDict()
        self._file_contents_cache: dict[str, bytes] = {}

    def _get_file_contents(self, file_path: str) -> bytes | None:
        """Get cached file contents."""
        if file_path in self._file_contents_cache:
            return self._file_contents_cache[file_path]
        try:
            content = Path(file_path).read_bytes()
            self._file_contents_cache[file_path] = content
            return content
        except Exception:
            return None

    def _file_contains_text(self, file_path: str, text: str) -> bool:
        """Check if a file contains the specified text without parsing AST."""
        content = self._get_file_contents(file_path)
        if content is None:
            return False
        return text.encode("utf-8") in content

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
        functions = []

        # Fast path optimization for simple queries
        is_simple_query = query and not any(c in query for c in ".^$*+?{}[]|()\\")

        for file_path in self.files:
            if is_simple_query and not self._file_contains_text(file_path, query):
                continue

            analyzer = self._get_analyzer(file_path)
            if analyzer:
                for f in analyzer.get_functions():
                    if match_query(f.name, query):
                        functions.append(f)
        return functions

    def get_classes(self, query: str = "") -> list[ClassInfo]:
        """Get all classes from all files."""
        classes = []

        # Fast path optimization for simple queries
        is_simple_query = query and not any(c in query for c in ".^$*+?{}[]|()\\")

        for file_path in self.files:
            if is_simple_query and not self._file_contains_text(file_path, query):
                continue

            analyzer = self._get_analyzer(file_path)
            if analyzer:
                for c in analyzer.get_classes():
                    if match_query(c.name, query):
                        classes.append(c)
        return classes

    def get_fields(self, class_name: str) -> list[FieldInfo]:
        """Get all fields from all files, optionally filtered by class name."""
        fields = []
        for file_path in self.files:
            if not self._file_contains_text(file_path, class_name):
                continue
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                fields.extend(analyzer.get_fields(class_name))
        return fields

    def get_calls(self) -> list[CallInfo]:
        """Get all function calls from all files."""
        calls = []
        for file_path in self.files:
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                calls.extend(analyzer.get_calls())
        return calls

    def get_imports(self, query: str = "") -> list[ImportInfo]:
        """Get all imports from all files."""
        imports = []

        # Fast path optimization for simple queries
        is_simple_query = query and not any(c in query for c in ".^$*+?{}[]|()\\")

        for file_path in self.files:
            if is_simple_query and not self._file_contains_text(file_path, query):
                continue

            analyzer = self._get_analyzer(file_path)
            if analyzer:
                for i in analyzer.get_imports():
                    if match_query(i.module, query):
                        imports.append(i)
        return imports

    def get_variables(self, query: str = "") -> list[VariableInfo]:
        """Get all variables from all files."""
        variables = []

        # Fast path optimization for simple queries
        is_simple_query = query and not any(c in query for c in ".^$*+?{}[]|()\\")

        for file_path in self.files:
            if is_simple_query and not self._file_contains_text(file_path, query):
                continue

            analyzer = self._get_analyzer(file_path)
            if analyzer:
                for v in analyzer.get_variables():
                    if match_query(v.name, query):
                        variables.append(v)
        return variables

    def get_function_by_name(self, name: str, class_name: str | None = None) -> FunctionInfo | None:
        """Find a function by name across all files, optionally filtering by class_name."""
        for file_path in self.files:
            if not self._file_contains_text(file_path, name):
                continue
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
        functions = []
        for file_path in self.files:
            if not self._file_contains_text(file_path, name):
                continue
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                funcs = analyzer.get_all_functions_by_name(name, class_name)
                functions.extend(funcs)
        return functions

    def get_callers(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Find all callers of a function across all files."""
        relevant_files = [f for f in self.files if self._file_contains_text(f, function_name)]

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
        relevant_files = [f for f in self.files if self._file_contains_text(f, function_name)]

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
        fn_variables = []
        for file_path in self.files:
            if not self._file_contains_text(file_path, function_name):
                continue
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                variables = analyzer.get_function_variables(function_name, class_name)
                for v in variables:
                    fn_variables.append(
                        {
                            "name": v.name,
                            "line": v.location.start_line,
                            "file": file_path,
                        }
                    )
        return sorted(fn_variables, key=lambda x: (x["file"], x["line"]))

    def get_function_strings(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Get all strings in a function across all files."""
        fn_strings = []
        for file_path in self.files:
            if not self._file_contains_text(file_path, function_name):
                continue
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                strings = analyzer.get_function_strings(function_name, class_name)
                for s in strings:
                    fn_strings.append(
                        {
                            "value": s.value,
                            "line": s.location.start_line,
                            "file": file_path,
                        }
                    )
        return sorted(fn_strings, key=lambda x: (x["file"], x["line"]))

    def find_symbols(self, name: str) -> list[dict]:
        """Find all references to an identifier across all files."""
        refs = []
        for file_path in self.files:
            if not self._file_contains_text(file_path, name):
                continue
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                file_refs = analyzer.find_symbols(name)
                refs.extend(file_refs)
        return refs

    def get_class_by_name(self, class_name: str) -> ClassInfo | None:
        """Find a class by name across all files."""
        for file_path in self.files:
            if not self._file_contains_text(file_path, class_name):
                continue
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

        while queue:
            current_class = queue.popleft()

            for parent_name in current_class.super_classes:
                if parent_name in visited:
                    continue

                # Try to find parent class definition
                parent_class = self.get_class_by_name(parent_name)
                if parent_class:
                    visited.add(parent_name)
                    result.append(parent_class)
                    queue.append(parent_class)
                else:
                    # Mark as visited even if not found to avoid repeated searches
                    visited.add(parent_name)

        return result

    def get_sub_classes(self, class_name: str) -> list[ClassInfo]:
        """Get all child classes (descendants) that inherit from a specific class using BFS.
        """
        result = []
        visited = {class_name}
        queue = deque([class_name])

        while queue:
            current_parent_name = queue.popleft()

            # Find direct subclasses of current_parent_name
            # Scan files that contain the parent name
            for file_path in self.files:
                if not self._file_contains_text(file_path, current_parent_name):
                    continue

                analyzer = self._get_analyzer(file_path)
                if not analyzer:
                    continue

                # Check all classes in this file
                for cls in analyzer.get_classes():
                    if cls.name in visited:
                        continue

                    if current_parent_name in cls.super_classes:
                        visited.add(cls.name)
                        result.append(cls)
                        queue.append(cls.name)

        return result
