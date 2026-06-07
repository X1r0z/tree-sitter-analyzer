use std::collections::HashSet;

use tree_sitter::Node;

use crate::languages::{LanguageEngine, ResolvedCall};
use crate::models::{FieldInfo, FunctionParamInfo};
use crate::parser::capture::CallCaptureMatch;
use crate::parser::{EnclosingContext, ParseContext};

mod attribute;
mod binding_events;
mod expression;
mod index;
mod resolvers;
mod semantic_facts;
mod type_helper;

pub(crate) use index::{JsReceiverIndex, JsTypeIndex};
pub(crate) use resolvers::{JsAliasResolver, JsReceiverResolver};
pub(crate) use semantic_facts::JsSemanticFacts;

use attribute::AttributeParts;
use binding_events::JsBindingEventCollector;
use index::type_index;
use type_helper::JsTypeHelper;

pub(crate) struct JavaScriptFamilyEngine {
    language: &'static str,
}

pub(crate) static JAVASCRIPT_ENGINE: JavaScriptFamilyEngine = JavaScriptFamilyEngine {
    language: "javascript",
};
pub(crate) static TYPESCRIPT_ENGINE: JavaScriptFamilyEngine = JavaScriptFamilyEngine {
    language: "typescript",
};
pub(crate) static TSX_ENGINE: JavaScriptFamilyEngine = JavaScriptFamilyEngine { language: "tsx" };

impl LanguageEngine for JavaScriptFamilyEngine {
    fn id(&self) -> &'static str {
        self.language
    }

    fn function_name(&self, ctx: &ParseContext, node: Node<'_>) -> Option<String> {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = ctx.node_text(name_node);
            if !name.is_empty() {
                return Some(name);
            }
        }
        if matches!(node.kind(), "arrow_function" | "function_expression") {
            return anonymous_function_name(ctx, node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(
                child.kind(),
                "identifier" | "property_identifier" | "field_identifier"
            ) {
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
        let type_helper = JsTypeHelper::new(ctx);
        let Some(parameters) = function_node.child_by_field_name("parameters") else {
            return Vec::new();
        };

        let mut params = Vec::new();
        let mut cursor = parameters.walk();
        for param in parameters.named_children(&mut cursor) {
            if let Some(info) = type_helper.build_param_info(param) {
                params.push(info);
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
        let call_node = matched.call;
        let mut callee = String::new();
        let mut is_method = false;
        let mut object_name: Option<String> = None;
        let mut callee_function_node: Option<Node<'_>> = None;

        match call_node.kind() {
            "call_expression" => {
                if let Some(func_node) = call_node.child_by_field_name("function") {
                    if func_node.kind() == "super" {
                        callee = "constructor".to_string();
                        object_name = Some("super()".to_string());
                    } else {
                        callee_function_node = Some(func_node);
                        if func_node.kind() == "identifier" {
                            callee = ctx.node_text(func_node);
                        } else if func_node.kind() == "member_expression" {
                            is_method = true;
                            let parts = AttributeParts::from_member_expression(ctx, func_node);
                            callee = parts.name;
                            object_name = parts.object;
                            if let Some(function_node) = enclosing.function_node {
                                if let Some(object_node) = func_node.child_by_field_name("object") {
                                    if let Some(receiver_class_name) =
                                        JsReceiverResolver::resolve_class_name(
                                            ctx,
                                            function_node,
                                            call_node,
                                            object_node,
                                        )
                                    {
                                        object_name = Some(receiver_class_name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "new_expression" => {
                if let Some(constructor_node) = call_node.child_by_field_name("constructor") {
                    callee = "constructor".to_string();
                    object_name = Some(ctx.node_text(constructor_node));
                }
            }
            _ => {}
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

    fn resolve_call_targets(
        &self,
        ctx: &ParseContext,
        function_node: Node<'_>,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        let function_id = ParseContext::node_id(function_node);
        let mut caches = ctx.caches.borrow_mut();
        let resolver = caches
            .language
            .js
            .alias_resolvers_by_function
            .entry(function_id)
            .or_insert_with(|| {
                JsAliasResolver::new(
                    JsBindingEventCollector::new(ctx).collect_alias_events(function_node),
                )
            });
        resolver.resolve(call_node.start_byte(), identifier_name)
    }

    fn is_ref_node(&self, node: Node<'_>) -> bool {
        matches!(
            node.kind(),
            "identifier"
                | "property_identifier"
                | "private_property_identifier"
                | "field_identifier"
                | "type_identifier"
        )
    }

    fn class_fields(
        &self,
        ctx: &ParseContext,
        _class_node: Node<'_>,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        type_index(ctx)
            .field_infos_by_class
            .get(class_name)
            .cloned()
            .unwrap_or_default()
    }

    fn super_classes(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
        let mut super_classes = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() != "class_heritage" {
                continue;
            }
            let mut child_cursor = child.walk();
            for sub in child.children(&mut child_cursor) {
                if matches!(sub.kind(), "extends_clause" | "implements_clause") {
                    let mut sub_cursor = sub.walk();
                    for grandchild in sub.children(&mut sub_cursor) {
                        if grandchild.is_named() {
                            collect_super_classes_from_node(
                                ctx,
                                grandchild,
                                &mut super_classes,
                                &mut seen,
                            );
                        }
                    }
                } else if sub.is_named() {
                    collect_super_classes_from_node(ctx, sub, &mut super_classes, &mut seen);
                }
            }
        }
        super_classes
    }
}

fn anonymous_function_name(ctx: &ParseContext, function_node: Node<'_>) -> Option<String> {
    let parent = function_node.parent()?;
    match parent.kind() {
        "variable_declarator" => {
            let name_node = parent.child_by_field_name("name")?;
            if name_node.kind() == "identifier" {
                return Some(ctx.node_text(name_node));
            }
        }
        "assignment_expression" | "assignment" => {
            let left_node = parent.child_by_field_name("left")?;
            if left_node.kind() == "identifier" {
                return Some(ctx.node_text(left_node));
            }
        }
        "pair" | "property" => {
            let key_node = parent.child_by_field_name("key")?;
            if matches!(
                key_node.kind(),
                "identifier" | "property_identifier" | "string"
            ) {
                let text = ctx.node_text_lossy(key_node);
                return Some(text.trim_matches(|c| c == '"' || c == '\'').to_string());
            }
        }
        "export_statement" => {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if ctx.node_text_eq(child, "default") {
                    return Some("<default_export>".to_string());
                }
            }
        }
        _ => {}
    }
    None
}

fn collect_super_classes_from_node(
    ctx: &ParseContext,
    node: Node<'_>,
    super_classes: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    match node.kind() {
        "identifier" | "type_identifier" | "property_identifier" => {
            let name = ctx.node_text(node).trim().to_string();
            if !name.is_empty() && seen.insert(name.clone()) {
                super_classes.push(name);
            }
        }
        "member_expression" => {
            let text = ctx.node_text(node).trim().to_string();
            if !text.is_empty() && seen.insert(text.clone()) {
                super_classes.push(text);
            }
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                if matches!(child.kind(), "identifier" | "property_identifier") {
                    let name = ctx.node_text(child).trim().to_string();
                    if !name.is_empty() && seen.insert(name.clone()) {
                        super_classes.push(name);
                    }
                    break;
                }
            }
        }
        "expression_with_type_arguments" => {
            if let Some(expr) = node.child_by_field_name("expression") {
                collect_super_classes_from_node(ctx, expr, super_classes, seen);
                return;
            }
            let mut cursor = node.walk();
            let first_child = node.named_children(&mut cursor).next();
            if let Some(child) = first_child {
                collect_super_classes_from_node(ctx, child, super_classes, seen);
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                collect_super_classes_from_node(ctx, function, super_classes, seen);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_super_classes_from_node(ctx, child, super_classes, seen);
            }
        }
    }
}
