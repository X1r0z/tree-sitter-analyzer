use tree_sitter::Node;

use super::super::{context, ParseContext};
use crate::models::{AnnotationInfo, FieldInfo, FunctionParamInfo};

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

        let type_node = param.child_by_field_name("type");
        let name = param
            .child_by_field_name("name")
            .map(|node| parser.node_text(node))
            .or_else(|| {
                (param.kind() == "receiver_parameter").then(|| {
                    parser
                        .node_text(param)
                        .split_whitespace()
                        .last()
                        .unwrap_or("")
                        .to_string()
                })
            })
            .unwrap_or_default();

        if name.is_empty() {
            continue;
        }

        params.push(FunctionParamInfo {
            name,
            param_type: type_node.map(|node| parser.node_text(node)),
        });
    }
    params
}

pub(crate) fn resolve_java_call_parts(
    parser: &ParseContext,
    call_node: Node<'_>,
) -> (String, bool, Option<String>) {
    if call_node.kind() == "explicit_constructor_invocation" {
        if let Some(constructor_node) = call_node.child_by_field_name("constructor") {
            return (parser.node_text(constructor_node), false, None);
        }
        return (String::new(), false, None);
    }

    if call_node.kind() == "object_creation_expression" {
        if let Some(type_node) = call_node.child_by_field_name("type") {
            if type_node.kind() == "generic_type" {
                for i in 0..type_node.named_child_count() {
                    let child = type_node.named_child(i as u32).unwrap();
                    if child.kind() == "type_identifier" {
                        return (parser.node_text(child), false, None);
                    }
                }
            } else {
                return (parser.node_text(type_node), false, None);
            }
        }
        return (String::new(), false, None);
    }

    let callee = call_node
        .child_by_field_name("name")
        .map(|node| parser.node_text(node))
        .unwrap_or_default();
    let object_name = call_node
        .child_by_field_name("object")
        .map(|node| parser.node_text(node));
    (callee, object_name.is_some(), object_name)
}

pub(crate) fn extract_java_signature(parser: &ParseContext, declaration_node: Node<'_>) -> String {
    let end_byte = declaration_node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or_else(|| declaration_node.end_byte());
    let start_byte = java_signature_start_byte(declaration_node);
    parser
        .source_text(start_byte, end_byte)
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

pub(crate) fn extract_java_class_header_line(
    parser: &ParseContext,
    declaration_node: Node<'_>,
) -> String {
    let signature = extract_java_signature(parser, declaration_node);
    signature
        .lines()
        .next()
        .unwrap_or("")
        .trim_end()
        .to_string()
}

fn java_signature_start_byte(declaration_node: Node<'_>) -> usize {
    for i in 0..declaration_node.child_count() {
        let Some(child) = declaration_node.child(i as u32) else {
            continue;
        };
        match child.kind() {
            "marker_annotation" | "annotation" => continue,
            "modifiers" => {
                let mut cursor = child.walk();
                for modifier_child in child.children(&mut cursor) {
                    if matches!(modifier_child.kind(), "marker_annotation" | "annotation") {
                        continue;
                    }
                    return modifier_child.start_byte();
                }
                continue;
            }
            _ => return child.start_byte(),
        }
    }

    declaration_node.start_byte()
}

pub(crate) fn extract_java_annotations(parser: &ParseContext) -> Vec<AnnotationInfo> {
    let mut annotations = Vec::new();
    let mut stack = vec![parser.tree.root_node()];

    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "marker_annotation" | "annotation") {
            if let Some(info) = java_annotation_info(parser, node) {
                annotations.push(info);
            }
            continue;
        }
        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i as u32) {
                stack.push(child);
            }
        }
    }
    annotations
}

fn java_annotation_info(parser: &ParseContext, node: Node<'_>) -> Option<AnnotationInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = parser.node_text(name_node);
    if name.is_empty() {
        return None;
    }

    let signature = parser.node_text(node);
    let (target_name, target_type, target_signature) = find_java_annotation_target(parser, node);

    Some(AnnotationInfo {
        name,
        signature,
        location: parser.node_location(node),
        target_name,
        target_type,
        target_signature,
    })
}

fn find_java_annotation_target(
    parser: &ParseContext,
    annotation_node: Node<'_>,
) -> (String, String, String) {
    let mut current = annotation_node;
    while let Some(parent) = current.parent() {
        let (target_name, target_type, target_signature) = match parent.kind() {
            "method_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "method".to_string(),
                format!(
                    "{}\n{}",
                    parser.node_text(annotation_node),
                    extract_java_signature(parser, parent)
                ),
            ),
            "constructor_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "constructor".to_string(),
                format!(
                    "{}\n{}",
                    parser.node_text(annotation_node),
                    extract_java_signature(parser, parent)
                ),
            ),
            "class_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "class".to_string(),
                format!(
                    "{}\n{}",
                    parser.node_text(annotation_node),
                    extract_java_class_header_line(parser, parent)
                ),
            ),
            "interface_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "interface".to_string(),
                format!(
                    "{}\n{}",
                    parser.node_text(annotation_node),
                    extract_java_class_header_line(parser, parent)
                ),
            ),
            "enum_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "enum".to_string(),
                format!(
                    "{}\n{}",
                    parser.node_text(annotation_node),
                    extract_java_class_header_line(parser, parent)
                ),
            ),
            "record_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "record".to_string(),
                format!(
                    "{}\n{}",
                    parser.node_text(annotation_node),
                    extract_java_class_header_line(parser, parent)
                ),
            ),
            "annotation_type_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "annotation_type".to_string(),
                format!(
                    "{}\n{}",
                    parser.node_text(annotation_node),
                    extract_java_class_header_line(parser, parent)
                ),
            ),
            "field_declaration" => {
                let field_name = declarator_name(parser, parent);
                (field_name, "field".to_string(), parser.node_text(parent))
            }
            "formal_parameter" | "spread_parameter" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "parameter".to_string(),
                find_enclosing_java_callable_signature(parser, parent),
            ),
            "local_variable_declaration" => {
                let var_name = declarator_name(parser, parent);
                (var_name, "variable".to_string(), parser.node_text(parent))
            }
            "modifiers" | "annotation_argument_list" => {
                current = parent;
                continue;
            }
            _ => {
                current = parent;
                continue;
            }
        };
        return (target_name, target_type, target_signature);
    }
    (String::new(), String::new(), String::new())
}

fn find_enclosing_java_callable_signature(parser: &ParseContext, node: Node<'_>) -> String {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if matches!(
            parent.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            return extract_java_signature(parser, parent);
        }
        current = parent;
    }
    String::new()
}

fn declarator_name(parser: &ParseContext, decl_node: Node<'_>) -> String {
    for i in 0..decl_node.child_count() {
        let child = decl_node.child(i as u32).unwrap();
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                return parser.node_text(name_node);
            }
        }
    }
    String::new()
}

pub(crate) fn field_infos(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
) -> Vec<FieldInfo> {
    context::collect_field_infos_from_declarations(parser, class_node, class_name, false)
}

pub(crate) fn super_class_names(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
    let mut super_classes = Vec::new();

    for i in 0..class_node.child_count() {
        let child = class_node.child(i as u32).unwrap();
        match child.kind() {
            "superclass" => {
                for j in 0..child.child_count() {
                    let sub = child.child(j as u32).unwrap();
                    if sub.kind() == "type_identifier" {
                        super_classes.push(parser.node_text(sub));
                    } else if sub.kind() == "generic_type" {
                        for k in 0..sub.child_count() {
                            let grandchild = sub.child(k as u32).unwrap();
                            if grandchild.kind() == "type_identifier" {
                                super_classes.push(parser.node_text(grandchild));
                                break;
                            }
                        }
                    }
                }
            }
            "super_interfaces" => {
                for j in 0..child.child_count() {
                    let sub = child.child(j as u32).unwrap();
                    if sub.kind() != "type_list" {
                        continue;
                    }
                    for k in 0..sub.child_count() {
                        let type_node = sub.child(k as u32).unwrap();
                        if type_node.kind() == "type_identifier" {
                            super_classes.push(parser.node_text(type_node));
                        } else if type_node.kind() == "generic_type" {
                            for l in 0..type_node.child_count() {
                                let grandchild = type_node.child(l as u32).unwrap();
                                if grandchild.kind() == "type_identifier" {
                                    super_classes.push(parser.node_text(grandchild));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    super_classes
}
