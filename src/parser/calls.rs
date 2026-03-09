use tree_sitter::Node;

use super::languages::java;
use super::query::{self, CallQueryMatch};
use super::ParseContext;
use crate::languages::QueryKind;
use crate::models::CallInfo;

impl ParseContext {
    pub(crate) fn collect_calls(&self) -> Vec<CallInfo> {
        let mut matched_calls: Vec<CallQueryMatch<'_>> =
            query::collect_call_matches(self, QueryKind::Call);
        let is_js_family = matches!(self.language.as_str(), "javascript" | "typescript" | "tsx");
        if is_js_family {
            matched_calls.sort_by_key(|m| m.call.start_byte());
        }

        let mut calls = Vec::new();
        for matched in &matched_calls {
            let call_node = matched.call;
            let enclosing = self.find_enclosing_context(call_node);
            let (callee, is_method, obj_name, callee_function_node) = if self.language == "java" {
                let (callee_name, resolved_is_method, object_name) =
                    java::resolve_java_call_parts(self, call_node);
                (callee_name, resolved_is_method, object_name, None)
            } else {
                resolve_non_java_call_parts(self, matched)
            };

            if callee.is_empty() {
                continue;
            }

            let call_location = self.node_location(call_node);
            let mut used_resolved_calls = false;
            if is_js_family
                && !is_method
                && callee_function_node
                    .map(|node| node.kind() == "identifier")
                    .unwrap_or(false)
                && enclosing.function_node.is_some()
            {
                let resolved = self.resolve_js_call_targets_for_identifier(call_node, &callee);
                if !resolved.is_empty() {
                    for resolved_callee in resolved {
                        calls.push(CallInfo {
                            callee: resolved_callee,
                            location: call_location.clone(),
                            caller: enclosing.function_name.clone(),
                            caller_class_name: enclosing.class_name.clone(),
                            object_name: obj_name.clone(),
                        });
                    }
                    used_resolved_calls = true;
                }
            }

            if !used_resolved_calls {
                calls.push(CallInfo {
                    callee,
                    location: call_location,
                    caller: enclosing.function_name.clone(),
                    caller_class_name: enclosing.class_name.clone(),
                    object_name: obj_name,
                });
            }
        }

        calls
    }
}

fn split_attribute_parts(parser: &ParseContext, node: Node<'_>) -> (String, Option<String>) {
    let mut callee = String::new();
    let mut obj_name: Option<String> = None;

    match node.kind() {
        "attribute" => {
            if let Some(attr_node) = node.child_by_field_name("attribute") {
                callee = parser.node_text(attr_node);
            }
            if let Some(obj_node) = node.child_by_field_name("object") {
                obj_name = Some(parser.node_text(obj_node));
            }
        }
        "member_expression" => {
            if let Some(prop_node) = node.child_by_field_name("property") {
                callee = parser.node_text(prop_node);
            }
            if let Some(obj_node) = node.child_by_field_name("object") {
                obj_name = Some(parser.node_text(obj_node));
            }
        }
        "selector_expression" => {
            if let Some(field_node) = node.child_by_field_name("field") {
                callee = parser.node_text(field_node);
            }
            if let Some(operand_node) = node.child_by_field_name("operand") {
                obj_name = Some(parser.node_text(operand_node));
            }
        }
        _ => {
            let mut ids = Vec::new();
            for i in 0..node.named_child_count() {
                let child = node.named_child(i as u32).unwrap();
                if matches!(
                    child.kind(),
                    "identifier"
                        | "property_identifier"
                        | "private_property_identifier"
                        | "field_identifier"
                ) {
                    ids.push(parser.node_text(child));
                } else if matches!(
                    child.kind(),
                    "attribute" | "member_expression" | "selector_expression"
                ) {
                    obj_name = Some(parser.node_text(child));
                }
            }
            if let Some(last) = ids.last() {
                callee = last.clone();
                if ids.len() > 1 && obj_name.is_none() {
                    obj_name = Some(ids[0].clone());
                }
            }
        }
    }

    (callee, obj_name)
}

fn resolve_non_java_call_parts<'a>(
    parser: &ParseContext,
    matched: &CallQueryMatch<'a>,
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
        } else if matches!(
            func_node.kind(),
            "attribute" | "member_expression" | "selector_expression"
        ) {
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
