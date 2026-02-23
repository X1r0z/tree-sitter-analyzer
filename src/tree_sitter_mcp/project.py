"""Project-level code analysis with directory support."""

from __future__ import annotations

import concurrent.futures
import contextlib
import re
from collections import OrderedDict, deque
from pathlib import Path

from .analyzer import (
    CallInfo,
    ClassInfo,
    CodeAnalyzer,
    FieldInfo,
    FunctionInfo,
    ImportInfo,
    VariableInfo,
)
from .languages import FILE_EXTENSION_MAP


def get_supported_extensions() -> set[str]:
    """Get all supported file extensions."""
    return set(FILE_EXTENSION_MAP.keys())


def _validate_directory_path(path: str) -> Path:
    p = Path(path)
    if not p.exists():
        raise FileNotFoundError(f"Path not found: {path}")
    if not p.is_dir():
        raise NotADirectoryError(f"Path must be a directory: {path}")

    return p


def find_files(path: str) -> list[str]:
    """Find all supported source files under a directory.

    Args:
        path: Directory path (searched recursively)

    Returns:
        List of absolute file paths
    """
    directory = _validate_directory_path(path)
    extensions = get_supported_extensions()
    files = [
        str(p.resolve())
        for p in directory.rglob("*")
        if p.is_file() and p.suffix.lower() in extensions
    ]
    return sorted(set(files))


def _process_file_for_callers(
    file_path: str, function_name: str, class_name: str | None
) -> list[dict]:
    """Worker function for parallel caller search."""
    try:
        analyzer = CodeAnalyzer(file_path)
        file_callers = analyzer.get_function_callers(function_name, class_name)
        results = []
        for caller_info in file_callers:
            results.append(
                {
                    "caller": caller_info["caller"],
                    "line": caller_info["line"],
                    "file": file_path,
                    "target_class": caller_info.get("target_class"),
                }
            )
        return results
    except Exception:
        return []


def _process_file_for_callees(
    file_path: str, function_name: str, class_name: str | None
) -> list[dict]:
    """Worker function for parallel callee search."""
    try:
        analyzer = CodeAnalyzer(file_path)
        # Check if function exists in file first (optimization inside worker)
        # Actually get_function_callees calls get_all_functions_by_name internally,
        # so we can just call get_function_callees directly if we trust it handles "not found" well.
        # But ProjectAnalyzer logic checked "if funcs" before calling get_function_callees.
        # Let's replicate the logic.
        funcs = analyzer.get_all_functions_by_name(function_name, class_name)
        if funcs:
            callees = analyzer.get_function_callees(function_name, class_name)
            results = []
            for c in callees:
                results.append(
                    {
                        "callee": c["callee"],
                        "line": c["line"],
                        "file": file_path,
                        "class_name": c.get("class_name"),
                    }
                )
            return results
        return []
    except Exception:
        return []


class ProjectAnalyzer:
    """Analyzes multiple source files in a project."""

    MAX_CACHED_ANALYZERS: int = 1000
    PARALLEL_THRESHOLD: int = 10

    def __init__(self, path: str):
        """Initialize with a directory.

        Args:
            path: Directory path (searched recursively)
        """
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

    def _match_query(self, name: str, query: str) -> bool:
        """Check if name matches query using regex."""
        if not query:
            return True
        try:
            return bool(re.search(query, name))
        except re.error:
            return query in name

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
                    if self._match_query(f.name, query):
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
                    if self._match_query(c.name, query):
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
                    if self._match_query(i.module, query):
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
                    if self._match_query(v.name, query):
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

        callers = []

        # Use sequential processing for small number of files to utilize cache and avoid overhead
        if len(relevant_files) < self.PARALLEL_THRESHOLD:
            for file_path in relevant_files:
                analyzer = self._get_analyzer(file_path)
                if analyzer:
                    file_callers = analyzer.get_function_callers(function_name, class_name)
                    for caller_info in file_callers:
                        callers.append(
                            {
                                "caller": caller_info["caller"],
                                "line": caller_info["line"],
                                "file": file_path,
                                "target_class": caller_info.get("target_class"),
                            }
                        )
        else:
            # Use parallel processing for larger number of files
            with concurrent.futures.ProcessPoolExecutor() as executor:
                futures = [
                    executor.submit(_process_file_for_callers, f, function_name, class_name)
                    for f in relevant_files
                ]
                for future in concurrent.futures.as_completed(futures):
                    with contextlib.suppress(Exception):
                        callers.extend(future.result())

        return sorted(callers, key=lambda x: (x["file"], x["line"]))

    def get_callees(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Find all functions called by a function across all files."""
        relevant_files = [f for f in self.files if self._file_contains_text(f, function_name)]

        if not relevant_files:
            return []

        results = []

        # Use sequential processing for small number of files
        if len(relevant_files) < self.PARALLEL_THRESHOLD:
            for file_path in relevant_files:
                analyzer = self._get_analyzer(file_path)
                if analyzer:
                    funcs = analyzer.get_all_functions_by_name(function_name, class_name)
                    if funcs:
                        callees = analyzer.get_function_callees(function_name, class_name)
                        for c in callees:
                            results.append(
                                {
                                    "callee": c["callee"],
                                    "line": c["line"],
                                    "file": file_path,
                                    "class_name": c.get("class_name"),
                                }
                            )
        else:
            # Use parallel processing for larger number of files
            with concurrent.futures.ProcessPoolExecutor() as executor:
                futures = [
                    executor.submit(_process_file_for_callees, f, function_name, class_name)
                    for f in relevant_files
                ]
                for future in concurrent.futures.as_completed(futures):
                    with contextlib.suppress(Exception):
                        results.extend(future.result())

        return sorted(results, key=lambda x: (x["file"], x["line"]))

    def get_function_variables(
        self, function_name: str, class_name: str | None = None
    ) -> list[dict]:
        """Get all variables in a function across all files."""
        results = []
        for file_path in self.files:
            if not self._file_contains_text(file_path, function_name):
                continue
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                variables = analyzer.get_function_variables(function_name, class_name)
                for v in variables:
                    results.append(
                        {
                            "name": v.name,
                            "line": v.location.start_line,
                            "file": file_path,
                        }
                    )
        return sorted(results, key=lambda x: (x["file"], x["line"]))

    def get_function_strings(self, function_name: str, class_name: str | None = None) -> list[dict]:
        """Get all strings in a function across all files."""
        results = []
        for file_path in self.files:
            if not self._file_contains_text(file_path, function_name):
                continue
            analyzer = self._get_analyzer(file_path)
            if analyzer:
                strings = analyzer.get_function_strings(function_name, class_name)
                for s in strings:
                    results.append(
                        {
                            "value": s.value,
                            "line": s.location.start_line,
                            "file": file_path,
                        }
                    )
        return sorted(results, key=lambda x: (x["file"], x["line"]))

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

        First finds the target class, then searches for its parent classes recursively.
        Uses text search pre-filtering to avoid parsing all files.
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

        Uses text search pre-filtering to avoid parsing all files.
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
