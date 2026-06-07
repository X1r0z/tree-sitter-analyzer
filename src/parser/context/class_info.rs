use std::collections::HashSet;

use tree_sitter::Node;

use crate::models::FieldInfo;

use super::ParseContext;

pub(crate) fn collect_class_fields(
    ctx: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
    include_embedded_type_names: bool,
) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![class_node];

    while let Some(node) = stack.pop() {
        // Field collection also treats a bare JS `class` expression as a boundary.
        if node.id() != class_node.id()
            && (is_nested_class_boundary(node.kind()) || node.kind() == "class")
        {
            let nested_name = node
                .child_by_field_name("name")
                .map(|child| ctx.node_text(child))
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
                .map(|child| ctx.node_text(child));

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier"
                ) {
                    names.push(ctx.node_text(child));
                } else if child.kind() == "variable_declarator" {
                    let mut child_cursor = child.walk();
                    for sub in child.children(&mut child_cursor) {
                        if sub.kind() == "identifier" {
                            names.push(ctx.node_text(sub));
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
                    field_type = Some(ctx.node_text(child));
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
                        location: ctx.node_location(node),
                        field_type: field_type.clone(),
                        class_name: Some(class_name.to_string()),
                    });
                }
            }
            continue;
        }

        push_children_reversed(&mut stack, node, false);
    }

    fields
}

pub(super) fn class_name(context: &ParseContext, node: Node<'_>) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let class_name = context.node_text(name_node);
        if !class_name.is_empty() {
            return Some(class_name);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "type_identifier" | "name") {
            let class_name = context.node_text(child);
            if !class_name.is_empty() {
                return Some(class_name);
            }
        }
    }
    None
}

pub(super) fn class_kind(context: &ParseContext, node: Node<'_>) -> String {
    match context.language() {
        "typescript" | "tsx" => match node.kind() {
            "abstract_class_declaration" => "abstract_class".to_string(),
            "interface_declaration" => "interface".to_string(),
            "type_alias_declaration" => "alias".to_string(),
            "enum_declaration" => "enum".to_string(),
            _ => "class".to_string(),
        },
        "java" => match node.kind() {
            "interface_declaration" => "interface".to_string(),
            "enum_declaration" => "enum".to_string(),
            "record_declaration" => "record".to_string(),
            "annotation_type_declaration" => "annotation".to_string(),
            _ => "class".to_string(),
        },
        "go" => node
            .children(&mut node.walk())
            .find(|child| child.kind() == "type_spec")
            .and_then(|type_spec| type_spec.child_by_field_name("type"))
            .map_or("struct", |type_node| match type_node.kind() {
                "interface_type" => "interface",
                _ => "struct",
            })
            .to_string(),
        _ => "class".to_string(),
    }
}

pub(super) fn class_methods(ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
    let mut methods = Vec::new();
    let mut stack = vec![class_node];
    while let Some(node) = stack.pop() {
        if node.id() != class_node.id() && is_nested_class_boundary(node.kind()) {
            continue;
        }
        if matches!(
            node.kind(),
            "function_definition"
                | "method_definition"
                | "method_declaration"
                | "constructor_declaration"
                | "method_elem"
                | "method_spec"
        ) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier" | "name"
                ) {
                    methods.push(ctx.node_text(child));
                    break;
                }
            }
            continue;
        }
        push_children_reversed(&mut stack, node, false);
    }
    methods
}

fn is_nested_class_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "class_definition"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration"
    )
}

/// Pushes `node`'s children onto `stack` in reverse so a stack-based DFS pops
/// them left-to-right. Set `named` to skip anonymous nodes.
pub(super) fn push_children_reversed<'a>(stack: &mut Vec<Node<'a>>, node: Node<'a>, named: bool) {
    let mut cursor = node.walk();
    let children: Vec<_> = if named {
        node.named_children(&mut cursor).collect()
    } else {
        node.children(&mut cursor).collect()
    };
    for child in children.into_iter().rev() {
        stack.push(child);
    }
}
