use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::super::{ParseContext, PythonPropertyCallers, PythonPropertyDefinitions};
use crate::models::{AnnotationInfo, FieldInfo, FunctionParamInfo, PythonPropertyCallerInfo};

type PythonPropertyCallerKey = (String, String, Option<String>, Option<String>, usize);

fn split_attribute_parts(parser: &ParseContext, node: Node<'_>) -> (String, Option<String>) {
    let mut callee = String::new();
    let mut obj_name: Option<String> = None;

    if let Some(attr_node) = node.child_by_field_name("attribute") {
        callee = parser.node_text(attr_node);
    }
    if let Some(obj_node) = node.child_by_field_name("object") {
        obj_name = Some(parser.node_text(obj_node));
    }

    (callee, obj_name)
}

pub(crate) fn python_function_params(
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
        if let Some(info) = build_python_parameter_info(parser, param) {
            params.push(info);
        }
    }
    params
}

fn build_python_parameter_info(
    parser: &ParseContext,
    param: Node<'_>,
) -> Option<FunctionParamInfo> {
    let type_node = param.child_by_field_name("type");
    let name = match param.kind() {
        "identifier" => parser.node_text(param),
        "typed_parameter" | "typed_default_parameter" | "default_parameter" => param
            .child_by_field_name("name")
            .or_else(|| param.child_by_field_name("pattern"))
            .or_else(|| param.child_by_field_name("left"))
            .map(|node| parser.node_text(node))
            .unwrap_or_else(|| first_python_identifier_text(parser, param)),
        "list_splat_pattern" | "dictionary_splat_pattern" => {
            parser.node_text(param).trim_start_matches('*').to_string()
        }
        _ => param
            .child_by_field_name("name")
            .or_else(|| param.child_by_field_name("pattern"))
            .or_else(|| param.child_by_field_name("left"))
            .map(|node| parser.node_text(node))
            .unwrap_or_else(|| first_python_identifier_text(parser, param)),
    };

    if name.is_empty() {
        return None;
    }

    Some(FunctionParamInfo {
        name,
        param_type: type_node.map(|node| parser.node_text(node)),
    })
}

pub(crate) fn extract_python_definition_header(
    parser: &ParseContext,
    definition_node: Node<'_>,
) -> String {
    let end_byte = definition_node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or_else(|| definition_node.end_byte());
    parser
        .source_text(definition_node.start_byte(), end_byte)
        .trim_end()
        .to_string()
}

pub(crate) fn extract_python_decorators(parser: &ParseContext) -> Vec<AnnotationInfo> {
    let mut annotations = Vec::new();
    let mut stack = vec![parser.tree.root_node()];

    while let Some(node) = stack.pop() {
        if node.kind() == "decorated_definition" {
            collect_python_decorators_from_decorated(parser, node, &mut annotations);
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
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
    annotations
}

fn collect_python_decorators_from_decorated(
    parser: &ParseContext,
    decorated_node: Node<'_>,
    annotations: &mut Vec<AnnotationInfo>,
) {
    let definition = decorated_node.child_by_field_name("definition");
    let (target_name, target_type, target_signature) = match definition {
        Some(definition_node) => {
            let name = definition_node
                .child_by_field_name("name")
                .map(|node| parser.node_text(node))
                .unwrap_or_default();
            let target_kind = match definition_node.kind() {
                "function_definition" => {
                    if parser
                        .find_enclosing_context(definition_node)
                        .class_name
                        .is_some()
                    {
                        "method"
                    } else {
                        "function"
                    }
                }
                "class_definition" => "class",
                _ => "",
            };
            (
                name,
                target_kind.to_string(),
                extract_python_definition_header(parser, definition_node),
            )
        }
        None => (String::new(), String::new(), String::new()),
    };

    for i in 0..decorated_node.child_count() {
        let child = decorated_node.child(i as u32).unwrap();
        if child.kind() != "decorator" {
            continue;
        }

        let name = extract_python_decorator_name(parser, child);
        if name.is_empty() {
            continue;
        }

        annotations.push(AnnotationInfo {
            name,
            signature: parser.node_text(child),
            location: parser.node_location(child),
            target_name: target_name.clone(),
            target_type: target_type.clone(),
            target_signature: format!("{}\n{}", parser.node_text(child), target_signature),
        });
    }
}

fn extract_python_decorator_name(parser: &ParseContext, decorator_node: Node<'_>) -> String {
    for i in 0..decorator_node.child_count() {
        let child = decorator_node.child(i as u32).unwrap();
        match child.kind() {
            "identifier" | "attribute" => return parser.node_text(child),
            "call" => {
                return child
                    .child_by_field_name("function")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default();
            }
            _ => {}
        }
    }
    String::new()
}

pub(crate) fn collect_python_property_indexes(
    parser: &ParseContext,
) -> (PythonPropertyDefinitions, PythonPropertyCallers) {
    if parser.language != "python" {
        return (HashSet::new(), HashMap::new());
    }

    let mut properties = HashSet::new();
    let mut callers_by_property: HashMap<String, Vec<PythonPropertyCallerInfo>> = HashMap::new();
    let mut seen_callers: HashSet<PythonPropertyCallerKey> = HashSet::new();
    let mut stack = vec![parser.tree.root_node()];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "decorated_definition" => {
                let Some(definition_node) = node.child_by_field_name("definition") else {
                    continue;
                };
                if definition_node.kind() != "function_definition" {
                    continue;
                }
                let Some(name_node) = definition_node.child_by_field_name("name") else {
                    continue;
                };

                let mut is_property = false;
                for i in 0..node.named_child_count() {
                    let Some(child) = node.named_child(i as u32) else {
                        continue;
                    };
                    if child.kind() == "decorator"
                        && parser.node_trimmed_text_eq(child, "@property")
                    {
                        is_property = true;
                        break;
                    }
                }

                if is_property {
                    properties.insert((
                        parser.node_text(name_node),
                        parser.find_enclosing_context(definition_node).class_name,
                    ));
                }
            }
            "attribute" => {
                let (property_name, object_name) = split_attribute_parts(parser, node);
                if property_name.is_empty() {
                    continue;
                }
                let enclosing = parser.find_enclosing_context(node);
                let caller = enclosing
                    .function_name
                    .unwrap_or_else(|| "<module>".to_string());
                let line = node.start_position().row + 1;
                let seen_key = (
                    property_name.clone(),
                    caller.clone(),
                    enclosing.class_name.clone(),
                    object_name.clone(),
                    line,
                );
                if seen_callers.insert(seen_key) {
                    callers_by_property
                        .entry(property_name.clone())
                        .or_default()
                        .push(PythonPropertyCallerInfo {
                            file: parser.file_path.clone(),
                            property_name: property_name.clone(),
                            caller,
                            caller_class_name: enclosing.class_name.clone(),
                            object_name,
                            line,
                        });
                }
            }
            _ => {}
        }

        for i in (0..node.named_child_count()).rev() {
            if let Some(child) = node.named_child(i as u32) {
                stack.push(child);
            }
        }
    }

    (properties, callers_by_property)
}

pub(crate) fn python_field_infos(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
) -> Vec<FieldInfo> {
    let fields = Vec::new();
    let seen = HashSet::new();

    struct WalkCtx<'a> {
        parser: &'a ParseContext,
        class_node_id: usize,
        class_name: String,
        fields: Vec<FieldInfo>,
        seen: HashSet<String>,
    }

    fn collect_field_infos(ctx: &mut WalkCtx, node: Node, inside_method: bool) {
        if node.id() != ctx.class_node_id
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
                .map(|child| ctx.parser.node_text(child))
                .unwrap_or_default();
            if !nested_name.is_empty() && nested_name != ctx.class_name {
                return;
            }
        }

        if matches!(
            node.kind(),
            "function_definition"
                | "method_definition"
                | "method_declaration"
                | "constructor_declaration"
        ) {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    collect_field_infos(ctx, child, true);
                }
            }
            return;
        }

        if node.kind() == "expression_statement" {
            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
                if child.kind() != "assignment" {
                    continue;
                }

                let Some(left_node) = child.child_by_field_name("left") else {
                    continue;
                };
                let field_type = child
                    .child_by_field_name("type")
                    .map(|node| ctx.parser.node_text(node));
                let mut name = String::new();

                if inside_method {
                    if left_node.kind() == "attribute" {
                        let obj_node = left_node.child_by_field_name("object");
                        let attr_node = left_node.child_by_field_name("attribute");
                        if let (Some(obj), Some(attr)) = (obj_node, attr_node) {
                            if ctx.parser.node_text_eq(obj, "self") {
                                name = ctx.parser.node_text(attr);
                            }
                        }
                    }
                } else if left_node.kind() == "identifier" {
                    name = ctx.parser.node_text(left_node);
                }

                if !name.is_empty() && ctx.seen.insert(name.clone()) {
                    ctx.fields.push(FieldInfo {
                        name,
                        location: ctx.parser.node_location(child),
                        field_type,
                        class_name: Some(ctx.class_name.clone()),
                    });
                }
            }
            return;
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                collect_field_infos(ctx, child, inside_method);
            }
        }
    }

    let mut ctx = WalkCtx {
        parser,
        class_node_id: class_node.id(),
        class_name: class_name.to_string(),
        fields,
        seen,
    };
    collect_field_infos(&mut ctx, class_node, false);
    ctx.fields
}

pub(crate) fn python_super_class_names(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
    let mut super_classes = Vec::new();
    for i in 0..class_node.child_count() {
        let child = class_node.child(i as u32).unwrap();
        if child.kind() != "argument_list" {
            continue;
        }
        for j in 0..child.child_count() {
            let arg = child.child(j as u32).unwrap();
            if matches!(arg.kind(), "identifier" | "attribute") {
                super_classes.push(parser.node_text(arg));
            }
        }
    }
    super_classes
}

fn first_python_identifier_text(parser: &ParseContext, node: Node<'_>) -> String {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if matches!(current.kind(), "identifier" | "keyword_identifier") {
            let text = parser.node_text(current);
            if !text.is_empty() {
                return text;
            }
        }
        for i in (0..current.named_child_count()).rev() {
            if let Some(child) = current.named_child(i as u32) {
                stack.push(child);
            }
        }
    }
    String::new()
}
