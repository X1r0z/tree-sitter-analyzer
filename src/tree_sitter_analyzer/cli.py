"""Command line interface for tree-sitter code analysis."""

from __future__ import annotations

import argparse
import json
import os
import sys

import yaml

from tree_sitter_mcp.project import ProjectAnalyzer


def _output_result(result: dict, output_format: str) -> None:
    """Output result in specified format."""
    if output_format == "json":
        print(json.dumps(result, indent=2, ensure_ascii=False))
    elif output_format == "yaml":
        print(yaml.dump(result, allow_unicode=True, default_flow_style=False, sort_keys=False))
    else:
        _print_pretty(result)


def _print_pretty(result: dict) -> None:
    """Print result in a human-readable format."""
    if "error" in result:
        print(f"Error: {result['error']}", file=sys.stderr)
        return

    path = result.get("path", "")
    count = result.get("count", 0)

    print(f"Path: {path}")
    if "files_searched" in result:
        print(f"Files searched: {result['files_searched']}")
    print(f"Count: {count}")
    print("-" * 50)

    if "functions" in result:
        for f in result["functions"]:
            loc = f"L{f['start_line']}-{f['end_line']}"
            file_info = f.get("file", "")
            class_info = f" ({f['class_name']})" if f.get("class_name") else ""
            method_tag = " [method]" if f.get("is_method") else ""
            if file_info:
                print(f"  {f['name']}{class_info}{method_tag} - {file_info}:{loc}")
            else:
                print(f"  {f['name']}{class_info}{method_tag} - {loc}")
            if "body" in f:
                print(f"    {f['body'][:100]}...")

    if "classes" in result:
        for c in result["classes"]:
            loc = f"L{c['start_line']}-{c['end_line']}"
            file_info = c.get("file", "")
            if file_info:
                print(f"  {c['name']} - {file_info}:{loc}")
            else:
                print(f"  {c['name']} - {loc}")
            if c.get("methods"):
                print(f"    Methods: {', '.join(c['methods'])}")
            if c.get("fields"):
                print(f"    Fields: {', '.join(c['fields'])}")

    if "fields" in result:
        for f in result["fields"]:
            type_info = f" ({f['type']})" if f.get("type") else ""
            file_info = f.get("file", "")
            if file_info:
                print(f"  {f['name']}{type_info} - {file_info}:L{f['line']}")
            else:
                print(f"  {f['name']}{type_info} - L{f['line']}")

    if "imports" in result:
        for i in result["imports"]:
            file_info = i.get("file", "")
            if file_info:
                print(f"  {i['module']} - {file_info}:L{i['line']}")
            else:
                print(f"  {i['module']} - L{i['line']}")

    if "variables" in result:
        for v in result["variables"]:
            scope = f" (scope: {v['scope']})" if v.get("scope") else " (global)"
            file_info = v.get("file", "")
            if file_info:
                print(f"  {v['name']}{scope} - {file_info}:L{v['line']}")
            else:
                print(f"  {v['name']}{scope} - L{v['line']}")

    if "callers" in result:
        func = result.get("function", "")
        print(f"Callers of '{func}':")
        for c in result["callers"]:
            file_info = c.get("file", "")
            if file_info:
                print(f"  {c['caller']} - {file_info}:L{c['line']}")
            else:
                print(f"  {c['caller']} - L{c['line']}")

    if "callees" in result:
        func = result.get("function", "")
        print(f"Callees of '{func}':")
        for c in result["callees"]:
            file_info = c.get("file", "")
            if file_info:
                print(f"  {c['callee']} - {file_info}:L{c['line']}")
            else:
                print(f"  {c['callee']} - L{c['line']}")

    if "references" in result:
        name = result.get("name", "")
        print(f"References to '{name}':")
        for r in result["references"]:
            loc = r.get("location", {})
            file_info = loc.get("file", "")
            line = loc.get("start_line", "?")
            if file_info:
                print(f"  {r['type']} - {file_info}:L{line}")
            else:
                print(f"  {r['type']} - L{line}")

    if "strings" in result:
        for s in result["strings"]:
            file_info = s.get("file", "")
            value = s["value"][:50] + "..." if len(s["value"]) > 50 else s["value"]
            if file_info:
                print(f"  {value} - {file_info}:L{s['line']}")
            else:
                print(f"  {value} - L{s['line']}")

    if "super_classes" in result:
        class_name = result.get("class_name", "")
        print(f"Super classes of '{class_name}':")
        for c in result["super_classes"]:
            file_info = c.get("file", "")
            if file_info:
                print(f"  {c['name']} - {file_info}:L{c['start_line']}")
            else:
                print(f"  {c['name']} - L{c['start_line']}")

    if "sub_classes" in result:
        class_name = result.get("class_name", "")
        print(f"Sub classes of '{class_name}':")
        for c in result["sub_classes"]:
            file_info = c.get("file", "")
            if file_info:
                print(f"  {c['name']} - {file_info}:L{c['start_line']}")
            else:
                print(f"  {c['name']} - L{c['start_line']}")


def cmd_functions(args: argparse.Namespace) -> dict:
    """Extract all function/method definitions."""
    try:
        path = os.path.realpath(args.path)
        query = args.query or ""
        project = ProjectAnalyzer(path)
        functions = project.get_functions(query=query)
        return {
            "path": path,
            "files_searched": len(project.files),
            "count": len(functions),
            "functions": [f.to_dict(include_body=args.body, include_file=True) for f in functions],
        }
    except Exception as e:
        return {"error": str(e)}


def cmd_classes(args: argparse.Namespace) -> dict:
    """Extract all class/struct/interface definitions."""
    try:
        path = os.path.realpath(args.path)
        query = args.query or ""
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


def cmd_fields(args: argparse.Namespace) -> dict:
    """Get all fields of a specific class."""
    try:
        path = os.path.realpath(args.path)
        class_name = args.class_name
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


def cmd_imports(args: argparse.Namespace) -> dict:
    """Extract all import statements."""
    try:
        path = os.path.realpath(args.path)
        query = args.query or ""
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


def cmd_variables(args: argparse.Namespace) -> dict:
    """Extract all variable declarations."""
    try:
        path = os.path.realpath(args.path)
        query = args.query or ""
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


def cmd_callers(args: argparse.Namespace) -> dict:
    """Find all functions that call a specific function."""
    try:
        path = os.path.realpath(args.path)
        function_name = args.function
        class_name = args.class_name
        project = ProjectAnalyzer(path)
        callers = project.get_callers(function_name, class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "function": function_name,
            "class_name": class_name,
            "count": len(callers),
            "callers": callers,
        }
    except Exception as e:
        return {"error": str(e)}


def cmd_callees(args: argparse.Namespace) -> dict:
    """Find all functions called by a specific function."""
    try:
        path = os.path.realpath(args.path)
        function_name = args.function
        class_name = args.class_name
        project = ProjectAnalyzer(path)
        callees = project.get_callees(function_name, class_name)
        return {
            "path": path,
            "files_searched": len(project.files),
            "function": function_name,
            "class_name": class_name,
            "count": len(callees),
            "callees": callees,
        }
    except Exception as e:
        return {"error": str(e)}


def cmd_symbols(args: argparse.Namespace) -> dict:
    """Find all references to a specific identifier."""
    try:
        path = os.path.realpath(args.path)
        name = args.name
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


def cmd_definition(args: argparse.Namespace) -> dict:
    """Get the complete definition of a specific function."""
    try:
        path = os.path.realpath(args.path)
        function_name = args.function
        class_name = args.class_name
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


def cmd_function_variables(args: argparse.Namespace) -> dict:
    """Get all variables declared within a specific function."""
    try:
        path = os.path.realpath(args.path)
        function_name = args.function
        class_name = args.class_name
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


def cmd_function_strings(args: argparse.Namespace) -> dict:
    """Get all string literals within a specific function."""
    try:
        path = os.path.realpath(args.path)
        function_name = args.function
        class_name = args.class_name
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


def cmd_super_classes(args: argparse.Namespace) -> dict:
    """Get all parent classes of a specific class."""
    try:
        path = os.path.realpath(args.path)
        class_name = args.class_name
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


def cmd_sub_classes(args: argparse.Namespace) -> dict:
    """Get all child classes that inherit from a specific class."""
    try:
        path = os.path.realpath(args.path)
        class_name = args.class_name
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


def create_parser() -> argparse.ArgumentParser:
    """Create the argument parser."""
    parser = argparse.ArgumentParser(
        prog="tree-sitter-analyzer",
        description="Tree-sitter based code analyzer for extracting code structure and relationships",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # List all classes in a directory
  tree-sitter-analyzer classes ./src/

  # List all functions in a directory
  tree-sitter-analyzer functions ./src/

  # Find all callers of a function
  tree-sitter-analyzer callers ./src/ --function process_data

  # Get function definition with body
  tree-sitter-analyzer definition ./src/ --function main

  # Output as JSON
  tree-sitter-analyzer functions ./src/ --json

Supported languages: Python, JavaScript/TypeScript, Java, Go
""",
    )

    parser.add_argument(
        "--version",
        action="version",
        version="%(prog)s 0.1.0",
    )

    subparsers = parser.add_subparsers(dest="command", help="Available commands")

    def add_format_args(p: argparse.ArgumentParser) -> None:
        p.add_argument("--json", action="store_true", help="Output in JSON format")
        p.add_argument("--yaml", action="store_true", help="Output in YAML format")

    # functions command
    p_functions = subparsers.add_parser("functions", help="Extract all function/method definitions")
    p_functions.add_argument("path", help="Directory path")
    p_functions.add_argument("-q", "--query", help="Filter by function name (regex match)")
    p_functions.add_argument("--body", action="store_true", help="Include function body")
    add_format_args(p_functions)
    p_functions.set_defaults(func=cmd_functions)

    # classes command
    p_classes = subparsers.add_parser(
        "classes", help="Extract all class/struct/interface definitions"
    )
    p_classes.add_argument("path", help="Directory path")
    p_classes.add_argument("-q", "--query", help="Filter by class name (regex match)")
    add_format_args(p_classes)
    p_classes.set_defaults(func=cmd_classes)

    # fields command
    p_fields = subparsers.add_parser("fields", help="Get all fields of a specific class")
    p_fields.add_argument("path", help="Directory path")
    p_fields.add_argument("-c", "--class-name", required=True, help="Class name to get fields for")
    add_format_args(p_fields)
    p_fields.set_defaults(func=cmd_fields)

    # imports command
    p_imports = subparsers.add_parser("imports", help="Extract all import statements")
    p_imports.add_argument("path", help="Directory path")
    p_imports.add_argument("-q", "--query", help="Filter by module name (regex match)")
    add_format_args(p_imports)
    p_imports.set_defaults(func=cmd_imports)

    # variables command
    p_variables = subparsers.add_parser("variables", help="Extract all variable declarations")
    p_variables.add_argument("path", help="Directory path")
    p_variables.add_argument("-q", "--query", help="Filter by variable name (regex match)")
    add_format_args(p_variables)
    p_variables.set_defaults(func=cmd_variables)

    # callers command
    p_callers = subparsers.add_parser(
        "callers", help="Find functions that call a specific function"
    )
    p_callers.add_argument("path", help="Directory path")
    p_callers.add_argument(
        "-f", "--function", required=True, help="Function name to find callers for"
    )
    p_callers.add_argument("-c", "--class-name", help="Class name to filter methods")
    add_format_args(p_callers)
    p_callers.set_defaults(func=cmd_callers)

    # callees command
    p_callees = subparsers.add_parser(
        "callees", help="Find functions called by a specific function"
    )
    p_callees.add_argument("path", help="Directory path")
    p_callees.add_argument(
        "-f", "--function", required=True, help="Function name to find callees for"
    )
    p_callees.add_argument("-c", "--class-name", help="Class name to filter methods")
    add_format_args(p_callees)
    p_callees.set_defaults(func=cmd_callees)

    # symbols command
    p_symbols = subparsers.add_parser(
        "symbols", help="Find all references to a specific identifier"
    )
    p_symbols.add_argument("path", help="Directory path")
    p_symbols.add_argument("-n", "--name", required=True, help="Identifier name to search for")
    add_format_args(p_symbols)
    p_symbols.set_defaults(func=cmd_symbols)

    # definition command
    p_definition = subparsers.add_parser(
        "definition", help="Get the complete source code of a function"
    )
    p_definition.add_argument("path", help="Directory path")
    p_definition.add_argument("-f", "--function", required=True, help="Function name to retrieve")
    p_definition.add_argument("-c", "--class-name", help="Class name to filter methods")
    add_format_args(p_definition)
    p_definition.set_defaults(func=cmd_definition)

    # function-variables command
    p_func_vars = subparsers.add_parser(
        "function-variables", help="Get all variables declared in a function"
    )
    p_func_vars.add_argument("path", help="Directory path")
    p_func_vars.add_argument("-f", "--function", required=True, help="Function name to analyze")
    p_func_vars.add_argument("-c", "--class-name", help="Class name to filter methods")
    add_format_args(p_func_vars)
    p_func_vars.set_defaults(func=cmd_function_variables)

    # function-strings command
    p_func_strings = subparsers.add_parser(
        "function-strings", help="Get all string literals in a function"
    )
    p_func_strings.add_argument("path", help="Directory path")
    p_func_strings.add_argument("-f", "--function", required=True, help="Function name to analyze")
    p_func_strings.add_argument("-c", "--class-name", help="Class name to filter methods")
    add_format_args(p_func_strings)
    p_func_strings.set_defaults(func=cmd_function_strings)

    # super-classes command
    p_super = subparsers.add_parser(
        "super-classes", help="Get all parent classes of a specific class"
    )
    p_super.add_argument("path", help="Directory path")
    p_super.add_argument("-c", "--class-name", required=True, help="Class name to find parents for")
    add_format_args(p_super)
    p_super.set_defaults(func=cmd_super_classes)

    # sub-classes command
    p_sub = subparsers.add_parser(
        "sub-classes", help="Get all child classes that inherit from a class"
    )
    p_sub.add_argument("path", help="Directory path")
    p_sub.add_argument("-c", "--class-name", required=True, help="Class name to find children for")
    add_format_args(p_sub)
    p_sub.set_defaults(func=cmd_sub_classes)

    return parser


def main() -> int:
    """Main entry point for the CLI."""
    parser = create_parser()
    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        return 1

    result = args.func(args)
    if args.json:
        output_format = "json"
    elif args.yaml:
        output_format = "yaml"
    else:
        output_format = "pretty"
    _output_result(result, output_format)

    return 0 if "error" not in result else 1


if __name__ == "__main__":
    sys.exit(main())
