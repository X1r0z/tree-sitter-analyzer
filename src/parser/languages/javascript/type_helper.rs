use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::models::FunctionParamInfo;
use crate::parser::ParseContext;

pub(super) struct JsTypeHelper<'a> {
    pub(super) ctx: &'a ParseContext,
}

impl<'a> JsTypeHelper<'a> {
    pub(super) fn new(ctx: &'a ParseContext) -> Self {
        Self { ctx }
    }

    pub(super) fn extract_class_targets(class_names: &HashSet<String>, field_type: &str) -> Vec<String> {
        let mut targets: Vec<String> = field_type
            .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
            .filter(|token| !token.is_empty())
            .filter(|candidate| class_names.contains(*candidate))
            .map(str::to_string)
            .collect();
        targets.sort_unstable();
        targets.dedup();
        targets
    }

    pub(super) fn field_type_from_value(&self, value_node: Node<'_>) -> Option<String> {
        match value_node.kind() {
            "call_expression" => {
                let function = value_node.child_by_field_name("function")?;
                let arguments = value_node.child_by_field_name("arguments")?;
                let is_supported_factory = matches!(
                    function.kind(),
                    "identifier" | "property_identifier" | "member_expression"
                ) && matches!(
                    self.ctx.node_text(function).as_str(),
                    "inject" | "forwardRef" | "signal" | "computed"
                );
                if !is_supported_factory {
                    return None;
                }
                self.first_type_name(arguments)
            }
            "new_expression" => value_node
                .child_by_field_name("constructor")
                .and_then(|constructor| self.type_name_from_node(constructor)),
            _ => None,
        }
    }

    pub(super) fn field_type_from_member(&self, member: Node<'_>, name_node_id: usize) -> Option<String> {
        let mut cursor = member.walk();
        for child in member.named_children(&mut cursor) {
            if child.id() == name_node_id || child.kind().ends_with("modifier") {
                continue;
            }
            if let Some(field_type) = self.field_type_from_value(child) {
                return Some(field_type);
            }
        }
        None
    }

    pub(super) fn field_type_from_constructor_param(
        &self,
        value_node: Node<'_>,
        class_name: &str,
        constructor_params: &[FunctionParamInfo],
        constructor_arg_types: &HashMap<(String, usize), Option<String>>,
    ) -> Option<String> {
        if value_node.kind() != "identifier" {
            return None;
        }
        let param_name = self.ctx.node_text(value_node);
        if param_name.is_empty() {
            return None;
        }

        if let Some(param_type) = constructor_params
            .iter()
            .find(|param| param.name == param_name)
            .and_then(|param| param.param_type.clone())
        {
            return Some(param_type);
        }

        let param_index = constructor_params
            .iter()
            .position(|param| param.name == param_name)?;
        constructor_arg_types
            .get(&(class_name.to_string(), param_index))
            .cloned()
            .unwrap_or(None)
    }

    fn first_type_name(&self, node: Node<'_>) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(name) = self.type_name_from_node(child) {
                return Some(name);
            }
        }
        None
    }

    pub(super) fn type_name_from_node(&self, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "identifier" | "type_identifier" => {
                let name = self.ctx.node_text(node);
                (!name.is_empty()).then_some(name)
            }
            "member_expression" => node
                .child_by_field_name("property")
                .and_then(|property| self.type_name_from_node(property)),
            "type_arguments" | "arguments" | "parenthesized_expression" => {
                self.first_type_name(node)
            }
            _ => None,
        }
    }

    pub(super) fn normalize_type_text(text: &str) -> String {
        let stripped = text.trim();
        if let Some(rest) = stripped.strip_prefix(':') {
            rest.trim().to_string()
        } else {
            stripped.to_string()
        }
    }

    pub(super) fn build_param_info(&self, param: Node<'_>) -> Option<FunctionParamInfo> {
        let name_node = Self::find_param_name_node(param)?;
        let name = self.ctx.node_text(name_node).trim().to_string();
        if name.is_empty() {
            return None;
        }

        Some(FunctionParamInfo {
            name,
            param_type: self.find_param_type(param),
        })
    }

    fn find_param_name_node(node: Node<'_>) -> Option<Node<'_>> {
        match node.kind() {
            "identifier"
            | "property_identifier"
            | "private_property_identifier"
            | "object_pattern"
            | "array_pattern" => Some(node),
            "assignment_pattern" => node
                .child_by_field_name("left")
                .and_then(Self::find_param_name_node),
            "rest_pattern" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    return Self::find_param_name_node(pattern);
                }
                node.named_child(0).and_then(Self::find_param_name_node)
            }
            _ => node
                .child_by_field_name("pattern")
                .or_else(|| node.child_by_field_name("name"))
                .and_then(Self::find_param_name_node)
                .or_else(|| node.named_child(0).and_then(Self::find_param_name_node)),
        }
    }

    fn find_param_type(&self, node: Node<'_>) -> Option<String> {
        if let Some(type_node) = node.child_by_field_name("type") {
            return Some(Self::normalize_type_text(&self.ctx.node_text(type_node)));
        }

        node.child_by_field_name("pattern")
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| node.child_by_field_name("left"))
            .and_then(|child| self.find_param_type(child))
    }

    pub(super) fn collect_constructor_params(&self, params_node: Node<'_>) -> Vec<FunctionParamInfo> {
        let mut params = Vec::new();
        let mut cursor = params_node.walk();
        for param in params_node.named_children(&mut cursor) {
            if let Some(info) = self.build_param_info(param) {
                params.push(info);
            }
        }
        params
    }
}
