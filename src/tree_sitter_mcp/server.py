"""Tree-sitter MCP Server for code analysis."""

from __future__ import annotations

import os

from fastmcp import FastMCP

from .project import ProjectAnalyzer

mcp = FastMCP(
    name="tree-sitter-mcp",
    instructions="""
    Tree-sitter MCP Server for code analysis.
    Provides function/class extraction, call graph analysis, inheritance analysis, and code structure analysis.
    Supported languages: Python, JavaScript, Java, Go
    """,
)


@mcp.tool
def get_functions(path: str, query: str = "") -> dict:
    """Extract all function/method definitions.

    Args:
        path: Directory path (searched recursively)
        query: Optional filter pattern for function/method names (regex match)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        functions = project.get_functions(query=query)
        return {
            "path": path,
            "files_searched": len(project.files),
            "count": len(functions),
            "functions": [f.to_dict(include_body=False, include_file=True) for f in functions],
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_classes(path: str, query: str = "") -> dict:
    """Extract all class/struct/interface definitions.

    Args:
        path: Directory path (searched recursively)
        query: Optional filter pattern for class names (regex match)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        classes = project.get_classes(query=query)
        return {
            "path": path,
            "files_searched": len(project.files),
            "count": len(classes),
            "classes": [c.to_dict(include_file=True) for c in classes],
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_fields(path: str, class_name: str) -> dict:
    """Get all fields of a specific class.

    Args:
        path: Directory path (searched recursively)
        class_name: Name of the class to get fields for
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        fields = project.get_fields(class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "class_name": class_name,
            "count": len(fields),
            "fields": [f.to_dict(include_file=True) for f in fields],
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_imports(path: str, query: str = "") -> dict:
    """Extract all import statements.

    Args:
        path: Directory path (searched recursively)
        query: Optional filter pattern for module names (regex match)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        imports = project.get_imports(query=query)
        return {
            "path": path,
            "files_searched": len(project.files),
            "count": len(imports),
            "imports": [i.to_dict(include_file=True) for i in imports],
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_variables(path: str, query: str = "") -> dict:
    """Extract all variable declarations.

    Args:
        path: Directory path (searched recursively)
        query: Optional filter pattern for variable names (regex match)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        variables = project.get_variables(query=query)
        return {
            "path": path,
            "files_searched": len(project.files),
            "count": len(variables),
            "variables": [v.to_dict(include_file=True) for v in variables],
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_callers(path: str, function_name: str, class_name: str | None = None) -> dict:
    """Find all functions that call a specific function.

    Args:
        path: Directory path (searched recursively)
        function_name: Name of the function to find callers for
        class_name: Optional class name to filter methods (if None, returns all matches)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        callers = project.get_callers(function_name, class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "function": function_name,
            "class_name": class_name,
            "callers": callers,
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_callees(path: str, function_name: str, class_name: str | None = None) -> dict:
    """Find all functions called by a specific function.

    Args:
        path: Directory path (searched recursively)
        function_name: Name of the function to find callees for
        class_name: Optional class name to filter methods (if None, returns all matches)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        callees = project.get_callees(function_name, class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "function": function_name,
            "class_name": class_name,
            "callees": callees,
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def find_symbols(path: str, name: str) -> dict:
    """Find all references to a specific identifier.

    Args:
        path: Directory path (searched recursively)
        name: Identifier name to search for
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        refs = project.find_symbols(name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "name": name,
            "count": len(refs),
            "references": refs,
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_function_definition(path: str, function_name: str, class_name: str | None = None) -> dict:
    """Get the complete definition (source code) of a specific function.

    Args:
        path: Directory path (searched recursively)
        function_name: Name of the function to retrieve
        class_name: Optional class name to filter methods (if None, returns all matches)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        functions = project.get_all_functions_by_name(function_name, class_name)
        if not functions:
            return {"error": f"Function '{function_name}' not found"}
        return {
            "path": path,
            "files_searched": len(project.files),
            "class_name": class_name,
            "count": len(functions),
            "functions": [f.to_dict() for f in functions],
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_function_variables(path: str, function_name: str, class_name: str | None = None) -> dict:
    """Get all variables declared within a specific function.

    Args:
        path: Directory path (searched recursively)
        function_name: Name of the function to analyze
        class_name: Optional class name to filter methods (if None, returns all matches)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        functions = project.get_all_functions_by_name(function_name, class_name)
        if not functions:
            return {"error": f"Function '{function_name}' not found"}
        variables = project.get_function_variables(function_name, class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "function": function_name,
            "class_name": class_name,
            "count": len(variables),
            "variables": variables,
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_function_strings(path: str, function_name: str, class_name: str | None = None) -> dict:
    """Get all string literals within a specific function.

    Args:
        path: Directory path (searched recursively)
        function_name: Name of the function to analyze
        class_name: Optional class name to filter methods (if None, returns all matches)
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        functions = project.get_all_functions_by_name(function_name, class_name)
        if not functions:
            return {"error": f"Function '{function_name}' not found"}
        strings = project.get_function_strings(function_name, class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "function": function_name,
            "class_name": class_name,
            "count": len(strings),
            "strings": strings,
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_super_classes(path: str, class_name: str) -> dict:
    """Get all parent classes (superclasses) of a specific class.

    Args:
        path: Directory path (searched recursively)
        class_name: Name of the class to find parent classes for
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        super_classes = project.get_super_classes(class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "class_name": class_name,
            "count": len(super_classes),
            "super_classes": [c.to_dict(include_file=True) for c in super_classes],
        }
    except Exception as e:
        return {"error": str(e)}


@mcp.tool
def get_sub_classes(path: str, class_name: str) -> dict:
    """Get all child classes (subclasses) that inherit from a specific class.

    Args:
        path: Directory path (searched recursively)
        class_name: Name of the class to find child classes for
    """
    try:
        path = os.path.realpath(path)
        project = ProjectAnalyzer(path)
        sub_classes = project.get_sub_classes(class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "class_name": class_name,
            "count": len(sub_classes),
            "sub_classes": [c.to_dict(include_file=True) for c in sub_classes],
        }
    except Exception as e:
        return {"error": str(e)}


def main():
    mcp.run()


if __name__ == "__main__":
    main()
