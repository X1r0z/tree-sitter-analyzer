use std::cmp::Reverse;
use std::collections::HashSet;

use tree_sitter::Node;

use super::languages::{go, java, javascript, python};
use super::query;
use super::ParseContext;
use crate::languages::QueryKind;
use crate::models::{ClassInfo, FieldInfo};

impl ParseContext {
    pub(crate) fn classes(&self) -> Vec<ClassInfo> {
        let mut methods_by_class = std::collections::HashMap::new();
        if self.language == "go" {
            for function in self.functions(false) {
                if let Some(class_name) = function.class_name {
                    methods_by_class
                        .entry(class_name)
                        .or_insert_with(Vec::new)
                        .push(function.name);
                }
            }
            for methods in methods_by_class.values_mut() {
                methods.sort_unstable();
                methods.dedup();
            }
        }

        let mut class_pairs: Vec<(Node<'_>, Node<'_>)> =
            query::query_capture_pairs(self, QueryKind::Class, "class", "name");
        class_pairs.sort_by_key(|(class_node, _)| {
            (class_node.start_byte(), Reverse(class_node.end_byte()))
        });

        let mut classes = Vec::new();
        let mut active_ranges: Vec<usize> = Vec::new();

        for (class_node, name_node) in class_pairs {
            let name = self.node_text(name_node);
            if name.is_empty() {
                continue;
            }
            let start = class_node.start_byte();
            let end = class_node.end_byte();
            while let Some(&active_end) = active_ranges.last() {
                if start >= active_end {
                    active_ranges.pop();
                } else {
                    break;
                }
            }
            let is_nested = !active_ranges.is_empty();
            if is_nested && self.language != "java" {
                continue;
            }
            active_ranges.push(end);

            let mut method_names = class_method_names(self, class_node);
            if self.language == "go" {
                if let Some(go_methods) = methods_by_class.get(&name) {
                    method_names.extend(go_methods.iter().cloned());
                    method_names.sort_unstable();
                    method_names.dedup();
                }
            }
            let field_names = class_field_names(self, class_node);
            let super_class_names = self.super_class_names(class_node);

            classes.push(ClassInfo {
                name,
                location: self.node_location(class_node),
                methods: method_names,
                fields: field_names,
                super_classes: super_class_names,
            });
        }

        classes
    }

    pub(crate) fn class_field_infos(&self, class_node: Node, class_name: &str) -> Vec<FieldInfo> {
        match self.language.as_str() {
            "python" => python::python_field_infos(self, class_node, class_name),
            "javascript" | "typescript" | "tsx" => {
                javascript::js_field_infos(self, class_node, class_name)
            }
            "java" => java::java_field_infos(self, class_node, class_name),
            "go" => go::go_field_infos(self, class_node, class_name),
            _ => Vec::new(),
        }
    }

    pub(crate) fn super_class_names(&self, class_node: Node) -> Vec<String> {
        match self.language.as_str() {
            "python" => python::python_super_class_names(self, class_node),
            "javascript" | "typescript" | "tsx" => {
                javascript::js_super_class_names(self, class_node)
            }
            "java" => java::java_super_class_names(self, class_node),
            "go" => go::go_embedded_type_names(self, class_node),
            _ => Vec::new(),
        }
    }

    pub(crate) fn field_infos_for_class(&self, class_name: &str) -> Vec<FieldInfo> {
        let mut candidates: Vec<Node<'_>> = Vec::new();
        for (class_node, name_node) in
            query::query_capture_pairs(self, QueryKind::Class, "class", "name")
        {
            if self.node_text_eq(name_node, class_name) {
                candidates.push(class_node);
            }
        }
        if candidates.is_empty() {
            return Vec::new();
        }

        candidates.sort_by_key(|node| {
            let size = node.end_byte() - node.start_byte();
            (size, node.start_byte())
        });
        self.class_field_infos(candidates[0], class_name)
    }
}

pub(crate) fn declared_field_infos(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
) -> Vec<FieldInfo> {
    collect_field_infos_from_declarations(parser, class_node, class_name, false)
}

pub(crate) fn declared_fields_with_embedded_types(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
) -> Vec<FieldInfo> {
    collect_field_infos_from_declarations(parser, class_node, class_name, true)
}

fn collect_field_infos_from_declarations(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
    include_embedded_type_names: bool,
) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![class_node];

    while let Some(node) = stack.pop() {
        if node.id() != class_node.id()
            && matches!(
                node.kind(),
                "class_definition"
                    | "class_declaration"
                    | "class"
                    | "interface_declaration"
                    | "enum_declaration"
                    | "record_declaration"
                    | "annotation_type_declaration"
            )
        {
            let nested_name = node
                .child_by_field_name("name")
                .map(|child| parser.node_text(child))
                .unwrap_or_default();
            if !nested_name.is_empty() && nested_name != class_name {
                continue;
            }
        }

        if matches!(
            node.kind(),
            "function_definition"
                | "method_definition"
                | "method_declaration"
                | "constructor_declaration"
        ) {
            continue;
        }

        if matches!(node.kind(), "field_definition" | "field_declaration") {
            let mut names = Vec::new();
            let mut field_type = node
                .child_by_field_name("type")
                .map(|child| parser.node_text(child));

            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier"
                ) {
                    names.push(parser.node_text(child));
                } else if child.kind() == "variable_declarator" {
                    for j in 0..child.child_count() {
                        let sub = child.child(j as u32).unwrap();
                        if sub.kind() == "identifier" {
                            names.push(parser.node_text(sub));
                            break;
                        }
                    }
                } else if field_type.is_none()
                    && matches!(
                        child.kind(),
                        "type_annotation"
                            | "type"
                            | "type_identifier"
                            | "integral_type"
                            | "floating_point_type"
                            | "boolean_type"
                            | "generic_type"
                            | "array_type"
                            | "scoped_type_identifier"
                    )
                {
                    field_type = Some(parser.node_text(child));
                }
            }

            if include_embedded_type_names && names.is_empty() {
                if let Some(field_type_text) = field_type.as_ref() {
                    let type_str = field_type_text.trim_start_matches('*');
                    let embedded_type_name = if type_str.contains('.') {
                        type_str.rsplit('.').next().unwrap_or(type_str)
                    } else {
                        type_str
                    };
                    names.push(embedded_type_name.to_string());
                }
            }

            for name in names {
                if !name.is_empty() && seen.insert(name.clone()) {
                    fields.push(FieldInfo {
                        name,
                        location: parser.node_location(node),
                        field_type: field_type.clone(),
                        class_name: Some(class_name.to_string()),
                    });
                }
            }
            continue;
        }

        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i as u32) {
                stack.push(child);
            }
        }
    }

    fields
}

fn class_method_names(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
    let mut methods = Vec::new();
    let mut stack = vec![class_node];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "function_definition"
                | "method_definition"
                | "method_declaration"
                | "constructor_declaration"
                | "method_elem"
                | "method_spec"
        ) {
            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier" | "name"
                ) {
                    methods.push(parser.node_text(child));
                    break;
                }
            }
            continue;
        }
        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i as u32) {
                stack.push(child);
            }
        }
    }
    methods
}

fn class_field_names(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
    let class_name = class_node
        .child_by_field_name("name")
        .map(|child| parser.node_text(child))
        .unwrap_or_default();
    parser
        .class_field_infos(class_node, &class_name)
        .into_iter()
        .map(|field| field.name)
        .collect()
}