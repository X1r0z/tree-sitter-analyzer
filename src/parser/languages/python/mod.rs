use tree_sitter::Node;

use super::super::capture::CallCaptureMatch;
use super::super::{EnclosingContext, ParseContext};
use crate::languages::{LanguageEngine, ResolvedCall};
use crate::models::{AnnotationInfo, FieldInfo, FunctionParamInfo};

mod annotations;
mod attribute;
mod fields;
mod params;
mod properties;

pub(crate) use properties::PythonPropertyAnalyzer;

use annotations::PythonAnnotationCollector;
use attribute::AttributeParts;
use fields::PythonFieldCollector;
use params::PythonParamHelper;

pub(crate) struct PythonEngine;

pub(crate) static PYTHON_ENGINE: PythonEngine = PythonEngine;

impl LanguageEngine for PythonEngine {
    fn id(&self) -> &'static str {
        "python"
    }

    fn normalize_function_node<'a>(&self, _ctx: &ParseContext, node: Node<'a>) -> Node<'a> {
        if node.kind() == "decorated_definition" {
            return node.child_by_field_name("definition").unwrap_or(node);
        }
        node
    }

    fn normalize_class_node<'a>(&self, _ctx: &ParseContext, node: Node<'a>) -> Node<'a> {
        if node.kind() == "decorated_definition" {
            return node.child_by_field_name("definition").unwrap_or(node);
        }
        node
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
            if child.kind() == "identifier" {
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
        PythonParamHelper::new(ctx).collect(function_node)
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

        if let Some(func_node) = call_node.child_by_field_name("function") {
            callee_function_node = Some(func_node);
            if func_node.kind() == "identifier" {
                callee = ctx.node_text(func_node);
            } else if func_node.kind() == "attribute" {
                is_method = true;
                let parts = AttributeParts::from_attribute(ctx, func_node);
                callee = parts.name;
                object_name = parts.object;
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
            "identifier" | "dotted_name" | "relative_import"
        )
    }

    fn class_fields(
        &self,
        ctx: &ParseContext,
        class_node: Node<'_>,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        PythonFieldCollector::new(ctx, class_node, class_name).collect()
    }

    fn super_classes(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
        let mut super_classes = Vec::new();
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() != "argument_list" {
                continue;
            }
            let mut child_cursor = child.walk();
            for arg in child.children(&mut child_cursor) {
                if matches!(arg.kind(), "identifier" | "attribute") {
                    super_classes.push(ctx.node_text(arg));
                }
            }
        }
        super_classes
    }

    fn annotations(&self, ctx: &ParseContext) -> Vec<AnnotationInfo> {
        PythonAnnotationCollector::new(ctx).collect()
    }

    fn include_call(
        &self,
        _ctx: &ParseContext,
        call_node: Node<'_>,
        callee: &str,
        object_name: Option<&str>,
    ) -> bool {
        if callee != "super" || object_name.is_some() {
            return true;
        }

        let Some(parent) = call_node.parent() else {
            return true;
        };
        !(parent.kind() == "attribute"
            && parent
                .child_by_field_name("object")
                .is_some_and(|object| object.id() == call_node.id()))
    }

    fn ref_filter(&self, _ctx: &ParseContext, node: Node<'_>) -> bool {
        if node.kind() != "identifier" {
            return true;
        }

        let mut current = node.parent();
        let mut has_import_name_ancestor = false;
        while let Some(parent) = current {
            match parent.kind() {
                "dotted_name" | "relative_import" => has_import_name_ancestor = true,
                "import_statement" | "import_from_statement" => return !has_import_name_ancestor,
                _ => {}
            }
            current = parent.parent();
        }

        true
    }
}
