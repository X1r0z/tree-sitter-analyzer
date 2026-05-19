use tree_sitter::Node;

use super::super::capture::CallCaptureMatch;
use super::super::{context, EnclosingContext, ParseContext};
use crate::languages::{LanguageEngine, ResolvedCall};
use crate::models::{FieldInfo, FunctionParamInfo};

pub(crate) struct GoEngine;

pub(crate) static GO_ENGINE: GoEngine = GoEngine;

impl LanguageEngine for GoEngine {
    fn id(&self) -> &'static str {
        "go"
    }

    fn function_name(&self, ctx: &ParseContext, node: Node<'_>) -> Option<String> {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = ctx.node_text(name_node);
            if !name.is_empty() {
                return Some(name);
            }
        }
        if node.kind() == "func_literal" {
            let parent = node.parent()?;
            if parent.kind() == "assignment_statement" || parent.kind() == "var_spec" {
                let mut cursor = parent.walk();
                for child in parent.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        let name = ctx.node_text(child);
                        if !name.is_empty() {
                            return Some(name);
                        }
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "identifier" | "field_identifier") {
                let name = ctx.node_text(child);
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
        None
    }

    fn function_params(
        &self,
        ctx: &ParseContext,
        function_node: Node<'_>,
    ) -> Vec<FunctionParamInfo> {
        let Some(parameters) = function_node.child_by_field_name("parameters") else {
            return Vec::new();
        };

        let mut params = Vec::new();
        let mut cursor = parameters.walk();
        for param in parameters.named_children(&mut cursor) {
            if !matches!(
                param.kind(),
                "parameter_declaration" | "variadic_parameter_declaration"
            ) {
                continue;
            }

            let param_type = param.child_by_field_name("type").map(|node| {
                let mut text = ctx.node_text(node);
                if param.kind() == "variadic_parameter_declaration" && !text.starts_with("...") {
                    text = format!("...{text}");
                }
                text
            });
            let mut names = Vec::new();

            let mut param_cursor = param.walk();
            for child in param.named_children(&mut param_cursor) {
                if child.kind() == "identifier" {
                    names.push(ctx.node_text(child));
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

    fn resolve_call<'a>(
        &self,
        ctx: &ParseContext,
        matched: &CallCaptureMatch<'a>,
        enclosing: &EnclosingContext<'a>,
    ) -> ResolvedCall<'a> {
        let _ = enclosing;
        let call_node = matched.call;
        let mut callee = String::new();
        let mut is_method = false;
        let mut object_name: Option<String> = None;
        let mut callee_function_node: Option<Node<'_>> = None;

        let function_node = call_node.child_by_field_name("function");
        if let Some(func_node) = function_node {
            callee_function_node = Some(func_node);
            match func_node.kind() {
                "identifier" => {
                    callee = ctx.node_text(func_node);
                }
                "selector_expression" => {
                    is_method = true;
                    let field_node = func_node.child_by_field_name("field");
                    let operand_node = func_node.child_by_field_name("operand");
                    if let Some(node) = field_node {
                        callee = ctx.node_text(node);
                    }
                    object_name = operand_node.map(|node| ctx.node_text(node));
                }
                _ => {}
            }
        }
        if callee.is_empty() {
            if let Some(callee_cap) = matched.callee {
                callee = ctx.node_text(callee_cap);
            }
            if let Some(method_cap) = matched.method {
                callee = ctx.node_text(method_cap);
                is_method = true;
                if let Some(obj_cap) = matched.object {
                    object_name = Some(ctx.node_text(obj_cap));
                }
            }
        }

        ResolvedCall {
            callee,
            is_method,
            object_name,
            callee_function_node,
        }
    }

    fn is_ref_node(&self, node: Node<'_>) -> bool {
        matches!(
            node.kind(),
            "identifier" | "field_identifier" | "type_identifier" | "qualified_type"
        )
    }

    fn class_fields(
        &self,
        ctx: &ParseContext,
        class_node: Node<'_>,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        context::collect_fields_from_declarations(ctx, class_node, class_name, true)
    }

    fn super_types(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
        let mut embedded_type_names = Vec::new();

        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() != "type_spec" {
                continue;
            }
            let mut child_cursor = child.walk();
            for sub in child.children(&mut child_cursor) {
                if sub.kind() != "struct_type" {
                    continue;
                }
                let mut sub_cursor = sub.walk();
                for field in sub.children(&mut sub_cursor) {
                    if field.kind() != "field_declaration_list" {
                        continue;
                    }
                    let mut field_cursor = field.walk();
                    for field_declaration in field.children(&mut field_cursor) {
                        if field_declaration.kind() != "field_declaration" {
                            continue;
                        }
                        if let Some(embedded) = embedded_type_from_field(ctx, field_declaration) {
                            embedded_type_names.push(embedded);
                        }
                    }
                }
            }
        }

        embedded_type_names
    }

    fn enclosing_class_name(&self, ctx: &ParseContext, node: Node<'_>) -> Option<String> {
        (node.kind() == "method_declaration")
            .then(|| receiver_type_name(ctx, node))
            .flatten()
    }
}

pub(crate) fn receiver_type_name(parser: &ParseContext, method_node: Node<'_>) -> Option<String> {
    let receiver_list = method_node.child_by_field_name("receiver").or_else(|| {
        let mut cursor = method_node.walk();
        let receiver = method_node
            .children(&mut cursor)
            .find(|child| child.kind() == "parameter_list");
        receiver
    })?;

    let mut cursor = receiver_list.walk();
    for param in receiver_list.children(&mut cursor) {
        if param.kind() != "parameter_declaration" {
            continue;
        }
        if let Some(type_node) = param.child_by_field_name("type") {
            return base_type_name(parser, type_node);
        }
        let mut param_cursor = param.walk();
        for child in param.children(&mut param_cursor) {
            if child.kind() == "pointer_type" {
                let mut child_cursor = child.walk();
                for pointer_child in child.children(&mut child_cursor) {
                    if pointer_child.kind() == "type_identifier" {
                        return Some(parser.node_text(pointer_child));
                    }
                }
            }
            if child.kind() == "type_identifier" {
                return Some(parser.node_text(child));
            }
        }
    }
    None
}

pub(crate) fn base_type_name(parser: &ParseContext, type_node: Node<'_>) -> Option<String> {
    match type_node.kind() {
        "type_identifier" => Some(parser.node_text(type_node)),
        "pointer_type" => {
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if child.kind() != "*" {
                    return base_type_name(parser, child);
                }
            }
            None
        }
        "generic_type" => {
            if let Some(base) = type_node.child_by_field_name("type") {
                return base_type_name(parser, base);
            }
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    return Some(parser.node_text(child));
                }
            }
            None
        }
        _ => None,
    }
}

fn embedded_type_name(parser: &ParseContext, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(parser.node_text(node)),
        "qualified_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" {
                    return Some(parser.node_text(child));
                }
            }
            None
        }
        "generic_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "type_identifier" | "qualified_type" | "pointer_type"
                ) {
                    return embedded_type_name(parser, child);
                }
            }
            None
        }
        "pointer_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "type_identifier" | "qualified_type" | "generic_type" | "parenthesized_type"
                ) {
                    return embedded_type_name(parser, child);
                }
            }
            None
        }
        "parenthesized_type" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if let Some(name) = embedded_type_name(parser, child) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

fn embedded_type_from_field(parser: &ParseContext, field_declaration: Node<'_>) -> Option<String> {
    let mut cursor = field_declaration.walk();
    for child in field_declaration.children(&mut cursor) {
        if child.kind() == "field_identifier" {
            return None;
        }
    }
    let mut cursor = field_declaration.walk();
    for child in field_declaration.children(&mut cursor) {
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
            return embedded_type_name(parser, child);
        }
    }
    let mut cursor = field_declaration.walk();
    for child in field_declaration.named_children(&mut cursor) {
        if let Some(name) = embedded_type_name(parser, child) {
            return Some(name);
        }
    }
    None
}
