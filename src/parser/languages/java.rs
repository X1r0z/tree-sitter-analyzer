use tree_sitter::Node;

use super::super::{context, EnclosingContext, ParseContext};
use crate::languages::{LanguageEngine, ResolvedCall};
use crate::models::{AnnotationInfo, FieldInfo, FunctionParamInfo};

pub(crate) struct JavaEngine;

pub(crate) static JAVA_ENGINE: JavaEngine = JavaEngine;

impl LanguageEngine for JavaEngine {
    fn id(&self) -> &'static str {
        "java"
    }

    fn function_name(&self, ctx: &ParseContext, node: Node<'_>) -> Option<String> {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = ctx.node_text(name_node);
            if !name.is_empty() {
                return Some(name);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
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
        let mut cursor = parameters.walk();
        for param in parameters.named_children(&mut cursor) {
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
        let (callee, is_method, object_name) = match call_node.kind() {
            "explicit_constructor_invocation" => (
                call_node
                    .child_by_field_name("constructor")
                    .map(|node| ctx.node_text(node))
                    .unwrap_or_default(),
                false,
                None,
            ),
            "object_creation_expression" => {
                let callee = call_node
                    .child_by_field_name("type")
                    .map(|type_node| {
                        if type_node.kind() != "generic_type" {
                            return ctx.node_text(type_node);
                        }

                        let mut cursor = type_node.walk();
                        for child in type_node.named_children(&mut cursor) {
                            if child.kind() == "type_identifier" {
                                return ctx.node_text(child);
                            }
                        }

                        String::new()
                    })
                    .unwrap_or_default();
                (callee, false, None)
            }
            _ => {
                let callee = call_node
                    .child_by_field_name("name")
                    .map(|node| ctx.node_text(node))
                    .unwrap_or_default();
                let object_node = call_node.child_by_field_name("object");
                let object_name = object_node.map(|node| {
                    let text = ctx.node_text(node);
                    if text == "super" {
                        "super()".to_string()
                    } else {
                        text
                    }
                });
                (callee, object_name.is_some(), object_name)
            }
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
        context::collect_fields_from_declarations(ctx, class_node, class_name, false)
    }

    fn super_types(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
        let mut super_classes = Vec::new();

        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            match child.kind() {
                "superclass" => {
                    let mut child_cursor = child.walk();
                    for sub in child.children(&mut child_cursor) {
                        if sub.kind() == "type_identifier" {
                            super_classes.push(ctx.node_text(sub));
                        } else if sub.kind() == "generic_type" {
                            let mut sub_cursor = sub.walk();
                            for grandchild in sub.children(&mut sub_cursor) {
                                if grandchild.kind() == "type_identifier" {
                                    super_classes.push(ctx.node_text(grandchild));
                                    break;
                                }
                            }
                        }
                    }
                }
                "super_interfaces" => {
                    let mut child_cursor = child.walk();
                    for sub in child.children(&mut child_cursor) {
                        if sub.kind() != "type_list" {
                            continue;
                        }
                        let mut sub_cursor = sub.walk();
                        for type_node in sub.children(&mut sub_cursor) {
                            if type_node.kind() == "type_identifier" {
                                super_classes.push(ctx.node_text(type_node));
                            } else if type_node.kind() == "generic_type" {
                                let mut type_cursor = type_node.walk();
                                for grandchild in type_node.children(&mut type_cursor) {
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
                let Some(name_node) = node.child_by_field_name("name") else {
                    continue;
                };

                let name = ctx.node_text(name_node);
                if name.is_empty() {
                    continue;
                }

                let signature = ctx.node_text(node);
                let (target_name, target_type, target_signature) =
                    find_annotation_target(ctx, node);

                annotations.push(AnnotationInfo {
                    name,
                    signature,
                    location: ctx.node_location(node),
                    target_name,
                    target_type,
                    target_signature,
                });
                continue;
            }
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
        annotations
    }
}

pub(crate) fn extract_signature(parser: &ParseContext, declaration_node: Node<'_>) -> String {
    let end_byte = declaration_node
        .child_by_field_name("body")
        .map_or_else(|| declaration_node.end_byte(), |body| body.start_byte());
    let start_byte = signature_start_byte(declaration_node);
    parser
        .source_text(start_byte, end_byte)
        .trim_end()
        .trim_end_matches(';')
        .trim_end()
        .to_string()
}

fn signature_start_byte(declaration_node: Node<'_>) -> usize {
    let mut cursor = declaration_node.walk();
    for child in declaration_node.children(&mut cursor) {
        match child.kind() {
            "marker_annotation" | "annotation" => {}
            "modifiers" => {
                let mut cursor = child.walk();
                for modifier_child in child.children(&mut cursor) {
                    if matches!(modifier_child.kind(), "marker_annotation" | "annotation") {
                        continue;
                    }
                    return modifier_child.start_byte();
                }
            }
            _ => return child.start_byte(),
        }
    }

    declaration_node.start_byte()
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
                    extract_signature(parser, parent)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim_end()
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
    let mut cursor = decl_node.walk();
    for child in decl_node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                return parser.node_text(name_node);
            }
        }
    }
    String::new()
}
