use std::cmp::Reverse;

use tree_sitter::Node;

use super::languages::{go, java, javascript, python};
use super::query;
use super::ParseContext;
use crate::languages::QueryKind;
use crate::models::{FunctionInfo, FunctionParamInfo};

impl ParseContext {
    pub(crate) fn collect_functions(&self, include_body: bool) -> Vec<FunctionInfo> {
        let mut func_pairs: Vec<(Node<'_>, Node<'_>)> =
            query::query_capture_pairs(self, QueryKind::Function, "function", "name");
        func_pairs.sort_by_key(|(f, _)| (f.start_byte(), Reverse(f.end_byte())));

        let mut functions = Vec::new();
        let mut active_ranges: Vec<usize> = Vec::new();

        for (func_node, name_node) in func_pairs {
            let name = self.node_text(name_node);
            if name.is_empty() {
                continue;
            }
            let start = func_node.start_byte();
            let end = func_node.end_byte();
            while let Some(&active_end) = active_ranges.last() {
                if start >= active_end {
                    active_ranges.pop();
                } else {
                    break;
                }
            }
            if !active_ranges.is_empty() {
                continue;
            }
            active_ranges.push(end);
            let class_name = self.find_enclosing_context(func_node).class_name;
            functions.push(FunctionInfo {
                name,
                location: self.node_location(func_node),
                body: if include_body {
                    self.node_text(func_node)
                } else {
                    String::new()
                },
                class_name,
                params: self.collect_function_params(func_node),
            });
        }

        functions
    }

    pub(crate) fn collect_function_params(&self, function_node: Node) -> Vec<FunctionParamInfo> {
        let target = unwrap_callable_node(function_node);
        match self.language.as_str() {
            "python" => python::python_function_params(self, target),
            "javascript" | "typescript" | "tsx" => javascript::js_function_params(self, target),
            "java" => java::java_function_params(self, target),
            "go" => go::go_function_params(self, target),
            _ => Vec::new(),
        }
    }
}

fn unwrap_callable_node<'a>(node: Node<'a>) -> Node<'a> {
    if node.kind() == "decorated_definition" {
        if let Some(definition) = node.child_by_field_name("definition") {
            return definition;
        }
    }
    node
}
