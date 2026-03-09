use tree_sitter::Node;

use super::super::capture::CallCaptureMatch;
use super::super::{context, ParseContext};
use crate::models::{FieldInfo, FunctionParamInfo};

pub(crate) fn function_params(
    parser: &ParseContext,
    function_node: Node<'_>,
) -> Vec<FunctionParamInfo> {
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
            let mut text = parser.node_text(node);
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
                names.push(parser.node_text(child));
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

pub(crate) fn receiver_type_name(parser: &ParseContext, method_node: Node<'_>) -> Option<String> {
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
            return base_type_name(parser, type_node);
        }
        for j in 0..param.child_count() {
            let child = param.child(j as u32).unwrap();
            if child.kind() == "pointer_type" {
                for k in 0..child.child_count() {
                    let pointer_child = child.child(k as u32).unwrap();
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

pub(crate) fn function_name_from_node(context: &ParseContext, node: Node<'_>) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = context.node_text(name_node);
        if !name.is_empty() {
            return Some(name);
        }
    }
    if node.kind() == "func_literal" {
        let parent = node.parent()?;
        if parent.kind() == "assignment_statement" || parent.kind() == "var_spec" {
            for i in 0..parent.child_count() {
                let child = parent.child(i as u32).unwrap();
                if child.kind() == "identifier" {
                    let name = context.node_text(child);
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }
    }
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if matches!(child.kind(), "identifier" | "field_identifier") {
            let name = context.node_text(child);
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

pub(crate) fn base_type_name(parser: &ParseContext, type_node: Node<'_>) -> Option<String> {
    match type_node.kind() {
        "type_identifier" => Some(parser.node_text(type_node)),
        "pointer_type" => {
            for i in 0..type_node.child_count() {
                let child = type_node.child(i as u32).unwrap();
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
            for i in 0..type_node.child_count() {
                let child = type_node.child(i as u32).unwrap();
                if child.kind() == "type_identifier" {
                    return Some(parser.node_text(child));
                }
            }
            None
        }
        _ => None,
    }
}

pub(crate) fn field_infos(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
) -> Vec<FieldInfo> {
    context::collect_field_infos_from_declarations(parser, class_node, class_name, true)
}

pub(crate) fn split_attribute_parts(
    parser: &ParseContext,
    node: Node<'_>,
) -> (String, Option<String>) {
    let mut callee = String::new();
    let mut obj_name: Option<String> = None;

    if let Some(field_node) = node.child_by_field_name("field") {
        callee = parser.node_text(field_node);
    }
    if let Some(operand_node) = node.child_by_field_name("operand") {
        obj_name = Some(parser.node_text(operand_node));
    }

    (callee, obj_name)
}

pub(crate) fn resolve_call_parts<'a>(
    parser: &ParseContext,
    matched: &CallCaptureMatch<'a>,
) -> (String, bool, Option<String>, Option<Node<'a>>) {
    let call_node = matched.call;
    let mut callee = String::new();
    let mut is_method = false;
    let mut obj_name: Option<String> = None;
    let mut callee_function_node: Option<Node<'_>> = None;

    if let Some(func_node) = call_node.child_by_field_name("function") {
        callee_function_node = Some(func_node);
        if func_node.kind() == "identifier" {
            callee = parser.node_text(func_node);
        } else if func_node.kind() == "selector_expression" {
            is_method = true;
            let (callee_name, object_name) = split_attribute_parts(parser, func_node);
            callee = callee_name;
            obj_name = object_name;
        }
    }
    if callee.is_empty() {
        if let Some(callee_cap) = matched.callee {
            callee = parser.node_text(callee_cap);
        }
        if let Some(method_cap) = matched.method {
            callee = parser.node_text(method_cap);
            is_method = true;
            if let Some(obj_cap) = matched.object {
                obj_name = Some(parser.node_text(obj_cap));
            }
        }
    }

    (callee, is_method, obj_name, callee_function_node)
}

pub(crate) fn is_ref_node(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "identifier" | "field_identifier" | "type_identifier" | "qualified_type"
    )
}

pub(crate) fn embedded_type_names(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
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
                        embedded_type_from_field_declaration(parser, field_declaration)
                    {
                        embedded_type_names.push(embedded);
                    }
                }
            }
        }
    }

    embedded_type_names
}

fn embedded_type_from_field_declaration(
    parser: &ParseContext,
    field_declaration: Node<'_>,
) -> Option<String> {
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
            return embedded_type_name(parser, child);
        }
    }
    for i in 0..field_declaration.named_child_count() {
        if let Some(child) = field_declaration.named_child(i as u32) {
            if let Some(name) = embedded_type_name(parser, child) {
                return Some(name);
            }
        }
    }
    None
}

fn embedded_type_name(parser: &ParseContext, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(parser.node_text(node)),
        "qualified_type" => {
            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
                if child.kind() == "type_identifier" {
                    return Some(parser.node_text(child));
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
                    return embedded_type_name(parser, child);
                }
            }
            None
        }
        "pointer_type" => {
            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
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
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i as u32) {
                    if let Some(name) = embedded_type_name(parser, child) {
                        return Some(name);
                    }
                }
            }
            None
        }
        _ => None,
    }
}
