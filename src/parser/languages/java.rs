use tree_sitter::Node;

use super::super::{context, EnclosingContext, ParseContext};
use crate::languages::{find_language_info, LanguageEngine, LanguageInfo, ResolvedCall};
use crate::models::{AnnotationInfo, FieldInfo, FunctionParamInfo};

pub(crate) struct JavaEngine;

pub(crate) static JAVA_ENGINE: JavaEngine = JavaEngine;

impl LanguageEngine for JavaEngine {
    fn language_info(&self) -> &'static LanguageInfo {
        find_language_info("java").expect("java language info")
    }

    fn function_name(&self, ctx: &ParseContext, node: Node<'_>) -> Option<String> {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = ctx.node_text(name_node);
            if !name.is_empty() {
                return Some(name);
            }
        }
        for i in 0..node.child_count() {
            let child = node.child(i as u32).unwrap();
            if matches!(child.kind(), "identifier" | "type_identifier") {
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
        for i in 0..parameters.named_child_count() {
            let Some(param) = parameters.named_child(i as u32) else {
                continue;
            };

            let type_node = param.child_by_field_name("type");
            let name = param
                .child_by_field_name("name")
                .map(|node| ctx.node_text(node))
                .or_else(|| {
                    (param.kind() == "receiver_parameter").then(|| {
                        ctx.node_text(param)
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
                param_type: type_node.map(|node| ctx.node_text(node)),
            });
        }
        params
    }

    fn resolve_call<'a>(
        &self,
        ctx: &ParseContext,
        matched: &super::super::capture::CallCaptureMatch<'a>,
        enclosing: &EnclosingContext<'a>,
    ) -> ResolvedCall<'a> {
        let _ = enclosing;
        let call_node = matched.call;
        let (callee, is_method, object_name) =
            if call_node.kind() == "explicit_constructor_invocation" {
                if let Some(constructor_node) = call_node.child_by_field_name("constructor") {
                    (ctx.node_text(constructor_node), false, None)
                } else {
                    (String::new(), false, None)
                }
            } else if call_node.kind() == "object_creation_expression" {
                if let Some(type_node) = call_node.child_by_field_name("type") {
                    if type_node.kind() == "generic_type" {
                        let mut callee = String::new();
                        for i in 0..type_node.named_child_count() {
                            let child = type_node.named_child(i as u32).unwrap();
                            if child.kind() == "type_identifier" {
                                callee = ctx.node_text(child);
                                break;
                            }
                        }
                        (callee, false, None)
                    } else {
                        (ctx.node_text(type_node), false, None)
                    }
                } else {
                    (String::new(), false, None)
                }
            } else {
                let callee = call_node
                    .child_by_field_name("name")
                    .map(|node| ctx.node_text(node))
                    .unwrap_or_default();
                let object_name = call_node.child_by_field_name("object").map(|node| {
                    if ctx.node_text_eq(node, "super") {
                        "super()".to_string()
                    } else {
                        ctx.node_text(node)
                    }
                });
                (callee, object_name.is_some(), object_name)
            };
        ResolvedCall {
            callee,
            is_method,
            object_name,
            callee_function_node: None,
        }
    }

    fn is_ref_node(&self, node: Node<'_>) -> bool {
        matches!(
            node.kind(),
            "identifier" | "type_identifier" | "scoped_identifier" | "scoped_type_identifier"
        )
    }

    fn class_fields(
        &self,
        ctx: &ParseContext,
        class_node: Node<'_>,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        context::collect_field_infos_from_declarations(ctx, class_node, class_name, false)
    }

    fn super_types(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
        let mut super_classes = Vec::new();

        for i in 0..class_node.child_count() {
            let child = class_node.child(i as u32).unwrap();
            match child.kind() {
                "superclass" => {
                    for j in 0..child.child_count() {
                        let sub = child.child(j as u32).unwrap();
                        if sub.kind() == "type_identifier" {
                            super_classes.push(ctx.node_text(sub));
                        } else if sub.kind() == "generic_type" {
                            for k in 0..sub.child_count() {
                                let grandchild = sub.child(k as u32).unwrap();
                                if grandchild.kind() == "type_identifier" {
                                    super_classes.push(ctx.node_text(grandchild));
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
                                super_classes.push(ctx.node_text(type_node));
                            } else if type_node.kind() == "generic_type" {
                                for l in 0..type_node.child_count() {
                                    let grandchild = type_node.child(l as u32).unwrap();
                                    if grandchild.kind() == "type_identifier" {
                                        super_classes.push(ctx.node_text(grandchild));
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

    fn annotations(&self, ctx: &ParseContext) -> Vec<AnnotationInfo> {
        let mut annotations = Vec::new();
        let mut stack = vec![ctx.tree().root_node()];

        while let Some(node) = stack.pop() {
            if matches!(node.kind(), "marker_annotation" | "annotation") {
                if let Some(info) = annotation_info(ctx, node) {
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
}

pub(crate) fn extract_signature(parser: &ParseContext, declaration_node: Node<'_>) -> String {
    let end_byte = declaration_node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or_else(|| declaration_node.end_byte());
    let start_byte = signature_start_byte(declaration_node);
    parser
        .source_text(start_byte, end_byte)
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

pub(crate) fn extract_class_header_line(
    parser: &ParseContext,
    declaration_node: Node<'_>,
) -> String {
    let signature = extract_signature(parser, declaration_node);
    signature
        .lines()
        .next()
        .unwrap_or("")
        .trim_end()
        .to_string()
}

fn signature_start_byte(declaration_node: Node<'_>) -> usize {
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

fn annotation_info(parser: &ParseContext, node: Node<'_>) -> Option<AnnotationInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = parser.node_text(name_node);
    if name.is_empty() {
        return None;
    }

    let signature = parser.node_text(node);
    let (target_name, target_type, target_signature) = find_annotation_target(parser, node);

    Some(AnnotationInfo {
        name,
        signature,
        location: parser.node_location(node),
        target_name,
        target_type,
        target_signature,
    })
}

fn find_annotation_target(
    parser: &ParseContext,
    annotation_node: Node<'_>,
) -> (String, String, String) {
    let annotation_signature = parser.node_text(annotation_node);
    let mut current = annotation_node;
    while let Some(parent) = current.parent() {
        let (target_name, target_type, target_signature) = match parent.kind() {
            "method_declaration" | "constructor_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                if parent.kind() == "method_declaration" {
                    "method".to_string()
                } else {
                    "constructor".to_string()
                },
                format!(
                    "{}\n{}",
                    annotation_signature,
                    extract_signature(parser, parent)
                ),
            ),
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                match parent.kind() {
                    "class_declaration" => "class",
                    "interface_declaration" => "interface",
                    "enum_declaration" => "enum",
                    "record_declaration" => "record",
                    "annotation_type_declaration" => "annotation_type",
                    _ => unreachable!(),
                }
                .to_string(),
                format!(
                    "{}\n{}",
                    annotation_signature,
                    extract_class_header_line(parser, parent)
                ),
            ),
            "field_declaration" => {
                let field_name = variable_declarator_name(parser, parent);
                (field_name, "field".to_string(), parser.node_text(parent))
            }
            "formal_parameter" | "spread_parameter" => (
                parent
                    .child_by_field_name("name")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default(),
                "parameter".to_string(),
                {
                    let mut current = parent;
                    let mut signature = String::new();
                    while let Some(ancestor) = current.parent() {
                        if matches!(
                            ancestor.kind(),
                            "method_declaration" | "constructor_declaration"
                        ) {
                            signature = extract_signature(parser, ancestor);
                            break;
                        }
                        current = ancestor;
                    }
                    signature
                },
            ),
            "local_variable_declaration" => {
                let var_name = variable_declarator_name(parser, parent);
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

fn variable_declarator_name(parser: &ParseContext, decl_node: Node<'_>) -> String {
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
