"""Utility functions for code analysis."""

from __future__ import annotations

import re
from functools import lru_cache
from pathlib import Path

import tree_sitter

from .languages import FILE_EXTENSION_MAP, get_language


@lru_cache(maxsize=128)
def get_compiled_query(language: str, query_str: str) -> tree_sitter.Query | None:
    """Get a cached compiled query for the given language and query string."""
    lang = get_language(language)
    if not lang:
        return None
    try:
        return tree_sitter.Query(lang, query_str)
    except Exception as e:
        print(f"Error compiling query for {language}: {e}")
        return None


def get_supported_extensions() -> set[str]:
    """Get all supported file extensions."""
    return set(FILE_EXTENSION_MAP.keys())


def validate_directory_path(path: str) -> Path:
    """Validate that path exists and is a directory."""
    p = Path(path)
    if not p.exists():
        raise FileNotFoundError(f"Path not found: {path}")
    if not p.is_dir():
        raise NotADirectoryError(f"Path must be a directory: {path}")
    return p


def find_files(path: str) -> list[str]:
    """Find all supported source files under a directory."""
    directory = validate_directory_path(path)
    extensions = get_supported_extensions()
    files = [
        str(p.resolve())
        for p in directory.rglob("*")
        if p.is_file() and p.suffix.lower() in extensions
    ]
    return sorted(set(files))


def match_query(name: str, query: str) -> bool:
    """Check if name matches query using regex."""
    if not query:
        return True
    try:
        return bool(re.search(query, name))
    except re.error:
        return query in name
