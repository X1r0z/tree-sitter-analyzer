"""Data structures for code analysis results."""

from __future__ import annotations

from dataclasses import dataclass, field

import tree_sitter


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

    def __getstate__(self) -> dict:
        state = self.__dict__.copy()
        state["node"] = None
        return state

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
