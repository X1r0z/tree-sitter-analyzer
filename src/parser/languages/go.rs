use tree_sitter::Node;

use super::super::ParseContext;
use crate::models::{FieldInfo, FunctionParamInfo};

impl ParseContext {
    pub(crate) fn go_function_params(&self, function_node: Node) -> Vec<FunctionParamInfo> {
        let Some(parameters) = function_node.child_by_field_name("parameters") else {
            return Vec::new();
        };

        let mut params = Vec::new();
        for i in 0..parameters.named_child_count() {
            let Some(param) = parameters.named_child(i as u32) else {
                continue;
            };
            if !matches!(
                param.kind(),
                "parameter_declaration" | "variadic_parameter_declaration"
            ) {
                continue;
            }

            let param_type = param.child_by_field_name("type").map(|node| {
                let mut text = self.node_text(node);
                if param.kind() == "variadic_parameter_declaration" && !text.starts_with("...") {
                    text = format!("...{text}");
                }
                text
            });
            let mut names = Vec::new();

            for j in 0..param.named_child_count() {
                let Some(child) = param.named_child(j as u32) else {
                    continue;
                };
                if child.kind() == "identifier" {
                    names.push(self.node_text(child));
                }
            }

            if names.is_empty() {
                params.push(FunctionParamInfo {
                    name: String::new(),
                    param_type,
                });
                continue;
            }

            for name in names {
                params.push(FunctionParamInfo {
                    name,
                    param_type: param_type.clone(),
                });
            }
        }
        params
    }

    pub(crate) fn go_receiver_type_name(&self, method_node: Node) -> Option<String> {
        let receiver_list = method_node.child_by_field_name("receiver").or_else(|| {
            for i in 0..method_node.child_count() {
                let child = method_node.child(i as u32).unwrap();
                if child.kind() == "parameter_list" {
                    return Some(child);
                }
            }
            None
        })?;

        for i in 0..receiver_list.child_count() {
            let param = receiver_list.child(i as u32).unwrap();
            if param.kind() != "parameter_declaration" {
                continue;
            }
            if let Some(type_node) = param.child_by_field_name("type") {
                return self.go_base_type_name(type_node);
            }
            for j in 0..param.child_count() {
                let child = param.child(j as u32).unwrap();
                if child.kind() == "pointer_type" {
                    for k in 0..child.child_count() {
                        let pointer_child = child.child(k as u32).unwrap();
                        if pointer_child.kind() == "type_identifier" {
                            return Some(self.node_text(pointer_child));
                        }
                    }
                }
                if child.kind() == "type_identifier" {
                    return Some(self.node_text(child));
                }
            }
        }
        None
    }

    pub(crate) fn go_base_type_name(&self, type_node: Node) -> Option<String> {
        match type_node.kind() {
            "type_identifier" => Some(self.node_text(type_node)),
            "pointer_type" => {
                for i in 0..type_node.child_count() {
                    let child = type_node.child(i as u32).unwrap();
                    if child.kind() != "*" {
                        return self.go_base_type_name(child);
                    }
                }
                None
            }
            "generic_type" => {
                if let Some(base) = type_node.child_by_field_name("type") {
                    return self.go_base_type_name(base);
                }
                for i in 0..type_node.child_count() {
                    let child = type_node.child(i as u32).unwrap();
                    if child.kind() == "type_identifier" {
                        return Some(self.node_text(child));
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn go_field_infos(&self, class_node: Node, class_name: &str) -> Vec<FieldInfo> {
        self.declared_fields_with_embedded_types(class_node, class_name)
    }

    pub(crate) fn go_embedded_type_names(&self, class_node: Node) -> Vec<String> {
        let mut embedded_type_names = Vec::new();

        for i in 0..class_node.child_count() {
            let child = class_node.child(i as u32).unwrap();
            if child.kind() != "type_spec" {
                continue;
            }
            for j in 0..child.child_count() {
                let sub = child.child(j as u32).unwrap();
                if sub.kind() != "struct_type" {
                    continue;
                }
                for k in 0..sub.child_count() {
                    let field = sub.child(k as u32).unwrap();
                    if field.kind() != "field_declaration_list" {
                        continue;
                    }
                    for l in 0..field.child_count() {
                        let field_declaration = field.child(l as u32).unwrap();
                        if field_declaration.kind() != "field_declaration" {
                            continue;
                        }
                        if let Some(embedded) =
                            self.embedded_type_from_field_declaration(field_declaration)
                        {
                            embedded_type_names.push(embedded);
                        }
                    }
                }
            }
        }

        embedded_type_names
    }

    fn embedded_type_from_field_declaration(&self, field_declaration: Node) -> Option<String> {
        for i in 0..field_declaration.child_count() {
            let child = field_declaration.child(i as u32).unwrap();
            if child.kind() == "field_identifier" {
                return None;
            }
        }
        for i in 0..field_declaration.child_count() {
            let child = field_declaration.child(i as u32).unwrap();
            if child.kind() == "*" {
                continue;
            }
            if matches!(
                child.kind(),
                "type_identifier"
                    | "qualified_type"
                    | "generic_type"
                    | "pointer_type"
                    | "parenthesized_type"
            ) {
                return self.embedded_type_name(child);
            }
        }
        for i in 0..field_declaration.named_child_count() {
            if let Some(child) = field_declaration.named_child(i as u32) {
                if let Some(name) = self.embedded_type_name(child) {
                    return Some(name);
                }
            }
        }
        None
    }

    fn embedded_type_name(&self, node: Node) -> Option<String> {
        match node.kind() {
            "type_identifier" => Some(self.node_text(node)),
            "qualified_type" => {
                for i in 0..node.child_count() {
                    let child = node.child(i as u32).unwrap();
                    if child.kind() == "type_identifier" {
                        return Some(self.node_text(child));
                    }
                }
                None
            }
            "generic_type" => {
                for i in 0..node.child_count() {
                    let child = node.child(i as u32).unwrap();
                    if matches!(
                        child.kind(),
                        "type_identifier" | "qualified_type" | "pointer_type"
                    ) {
                        return self.embedded_type_name(child);
                    }
                }
                None
            }
            "pointer_type" => {
                for i in 0..node.child_count() {
                    let child = node.child(i as u32).unwrap();
                    if matches!(
                        child.kind(),
                        "type_identifier"
                            | "qualified_type"
                            | "generic_type"
                            | "parenthesized_type"
                    ) {
                        return self.embedded_type_name(child);
                    }
                }
                None
            }
            "parenthesized_type" => {
                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child(i as u32) {
                        if let Some(name) = self.embedded_type_name(child) {
                            return Some(name);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}
