"""Top-level worker functions for ProcessPoolExecutor (must be picklable)."""

from __future__ import annotations

from .analyzer import CodeAnalyzer
from .nodes import (
    CallInfo,
    ClassInfo,
    FieldInfo,
    FunctionInfo,
    ImportInfo,
    VariableInfo,
)
from .utils import match_query


def find_functions(file_path: str, q: str) -> list[FunctionInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return [f for f in analyzer.get_functions() if match_query(f.name, q)]
    except Exception:
        return []


def find_classes(file_path: str, q: str) -> list[ClassInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return [c for c in analyzer.get_classes() if match_query(c.name, q)]
    except Exception:
        return []


def find_fields(file_path: str, cls_name: str) -> list[FieldInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return analyzer.get_fields(cls_name)
    except Exception:
        return []


def find_calls(file_path: str) -> list[CallInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return analyzer.get_calls()
    except Exception:
        return []


def find_imports(file_path: str, q: str) -> list[ImportInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return [i for i in analyzer.get_imports() if match_query(i.module, q)]
    except Exception:
        return []


def find_variables(file_path: str, q: str) -> list[VariableInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return [v for v in analyzer.get_variables() if match_query(v.name, q)]
    except Exception:
        return []


def find_funcs_by_name(
    file_path: str,
    fn_name: str,
    cls_name: str | None,
) -> list[FunctionInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return analyzer.get_all_functions_by_name(fn_name, cls_name)
    except Exception:
        return []


def find_callers(
    file_path: str,
    fn_name: str,
    cls_name: str | None,
) -> list[dict]:
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


def find_callees(
    file_path: str,
    fn_name: str,
    cls_name: str | None,
) -> list[dict]:
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


def find_fn_vars(
    file_path: str,
    fn_name: str,
    cls_name: str | None,
) -> list[dict]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return [
            {"name": v.name, "line": v.location.start_line, "file": file_path}
            for v in analyzer.get_function_variables(fn_name, cls_name)
        ]
    except Exception:
        return []


def find_fn_strings(
    file_path: str,
    fn_name: str,
    cls_name: str | None,
) -> list[dict]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return [
            {"value": s.value, "line": s.location.start_line, "file": file_path}
            for s in analyzer.get_function_strings(fn_name, cls_name)
        ]
    except Exception:
        return []


def find_refs(file_path: str, symbol_name: str) -> list[dict]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return analyzer.find_symbols(symbol_name)
    except Exception:
        return []


def find_class_def(file_path: str, cls_name: str) -> list[ClassInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        cls = analyzer.get_class_by_name(cls_name)
        return [cls] if cls else []
    except Exception:
        return []


def find_subclasses(file_path: str, parent_name: str) -> list[ClassInfo]:
    try:
        analyzer = CodeAnalyzer(file_path)
        return [cls for cls in analyzer.get_classes() if parent_name in cls.super_classes]
    except Exception:
        return []
