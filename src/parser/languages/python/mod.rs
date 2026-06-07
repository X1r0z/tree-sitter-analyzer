use std::collections::HashSet;

use tree_sitter::Node;

use super::super::capture::CallCaptureMatch;
use super::super::{EnclosingContext, ParseContext};
use crate::languages::{LanguageEngine, ResolvedCall};
use crate::models::{AnnotationInfo, FieldInfo, FunctionParamInfo};

mod property_analyzer;

pub(crate) use property_analyzer::PythonPropertyAnalyzer;

pub(crate) struct PythonEngine;

pub(crate) static PYTHON_ENGINE: PythonEngine = PythonEngine;

struct AttributeParts {
    name: String,
    object: Option<String>,
}

struct PythonParamHelper<'a> {
    parser: &'a ParseContext,
}

struct PythonFieldCollector<'a> {
    parser: &'a ParseContext,
    class_node: Node<'a>,
    class_node_id: usize,
    class_name: String,
    fields: Vec<FieldInfo>,
    seen: HashSet<String>,
}

struct PythonAnnotationCollector<'a> {
    parser: &'a ParseContext,
}

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

impl AttributeParts {
    fn from_attribute(parser: &ParseContext, node: Node<'_>) -> Self {
        let name = node
            .child_by_field_name("attribute")
            .map(|attr_node| parser.node_text(attr_node))
            .unwrap_or_default();
        let object = node
            .child_by_field_name("object")
            .map(|obj_node| parser.node_text(obj_node));

        Self { name, object }
    }
}

impl<'a> PythonParamHelper<'a> {
    fn new(parser: &'a ParseContext) -> Self {
        Self { parser }
    }

    fn collect(&self, function_node: Node<'_>) -> Vec<FunctionParamInfo> {
        let Some(parameters) = function_node.child_by_field_name("parameters") else {
            return Vec::new();
        };

        let mut params = Vec::new();
        let mut cursor = parameters.walk();
        for param in parameters.named_children(&mut cursor) {
            if let Some(info) = self.build_param_info(param) {
                params.push(info);
            }
        }
        params
    }

    fn build_param_info(&self, param: Node<'_>) -> Option<FunctionParamInfo> {
        let type_node = param.child_by_field_name("type");
        let name = match param.kind() {
            "identifier" => self.parser.node_text(param),
            "typed_parameter" | "typed_default_parameter" | "default_parameter" => param
                .child_by_field_name("name")
                .or_else(|| param.child_by_field_name("pattern"))
                .or_else(|| param.child_by_field_name("left"))
                .map_or_else(
                    || self.first_identifier_text(param),
                    |node| self.parser.node_text(node),
                ),
            "list_splat_pattern" | "dictionary_splat_pattern" => self
                .parser
                .node_text(param)
                .trim_start_matches('*')
                .to_string(),
            _ => param
                .child_by_field_name("name")
                .or_else(|| param.child_by_field_name("pattern"))
                .or_else(|| param.child_by_field_name("left"))
                .map_or_else(
                    || self.first_identifier_text(param),
                    |node| self.parser.node_text(node),
                ),
        };

        if name.is_empty() {
            return None;
        }

        Some(FunctionParamInfo {
            name,
            param_type: type_node.map(|node| self.parser.node_text(node)),
        })
    }

    fn first_identifier_text(&self, node: Node<'_>) -> String {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if matches!(current.kind(), "identifier" | "keyword_identifier") {
                let text = self.parser.node_text(current);
                if !text.is_empty() {
                    return text;
                }
            }
            let mut cursor = current.walk();
            let children: Vec<_> = current.named_children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
        String::new()
    }
}

impl<'a> PythonFieldCollector<'a> {
    fn new(parser: &'a ParseContext, class_node: Node<'a>, class_name: &str) -> Self {
        Self {
            parser,
            class_node,
            class_node_id: class_node.id(),
            class_name: class_name.to_string(),
            fields: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn collect(mut self) -> Vec<FieldInfo> {
        self.walk(self.class_node, false);
        self.fields
    }

    fn walk(&mut self, node: Node<'a>, inside_method: bool) {
        if node.id() != self.class_node_id
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
                .map(|child| self.parser.node_text(child))
                .unwrap_or_default();
            if !nested_name.is_empty() && nested_name != self.class_name {
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
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk(child, true);
            }
            return;
        }

        if node.kind() == "expression_statement" {
            self.collect_assignment_fields(node, inside_method);
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, inside_method);
        }
    }

    fn collect_assignment_fields(&mut self, node: Node<'a>, inside_method: bool) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "assignment" {
                continue;
            }

            let Some(left_node) = child.child_by_field_name("left") else {
                continue;
            };
            let field_type = child
                .child_by_field_name("type")
                .map(|node| self.parser.node_text(node));
            let mut name = String::new();

            if inside_method {
                if left_node.kind() == "attribute" {
                    let obj_node = left_node.child_by_field_name("object");
                    let attr_node = left_node.child_by_field_name("attribute");
                    if let (Some(obj), Some(attr)) = (obj_node, attr_node) {
                        if self.parser.node_text_eq(obj, "self") {
                            name = self.parser.node_text(attr);
                        }
                    }
                }
            } else if left_node.kind() == "identifier" {
                name = self.parser.node_text(left_node);
            }

            if !name.is_empty() && self.seen.insert(name.clone()) {
                self.fields.push(FieldInfo {
                    name,
                    location: self.parser.node_location(child),
                    field_type,
                    class_name: Some(self.class_name.clone()),
                });
            }
        }
    }
}

impl<'a> PythonAnnotationCollector<'a> {
    fn new(parser: &'a ParseContext) -> Self {
        Self { parser }
    }

    fn collect(&self) -> Vec<AnnotationInfo> {
        let mut annotations = Vec::new();
        let mut stack = vec![self.parser.tree().root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "decorated_definition" {
                self.collect_from_decorated(node, &mut annotations);
                let mut cursor = node.walk();
                let children: Vec<_> = node.children(&mut cursor).collect();
                for child in children.into_iter().rev() {
                    stack.push(child);
                }
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

    fn collect_from_decorated(
        &self,
        decorated_node: Node<'_>,
        annotations: &mut Vec<AnnotationInfo>,
    ) {
        let definition = decorated_node.child_by_field_name("definition");
        let (target_name, target_type, target_signature) = match definition {
            Some(definition_node) => {
                let name = definition_node
                    .child_by_field_name("name")
                    .map(|node| self.parser.node_text(node))
                    .unwrap_or_default();
                let target_kind = match definition_node.kind() {
                    "function_definition" => {
                        if self
                            .parser
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
                    self.definition_header(definition_node),
                )
            }
            None => (String::new(), String::new(), String::new()),
        };

        let mut cursor = decorated_node.walk();
        for child in decorated_node.children(&mut cursor) {
            if child.kind() != "decorator" {
                continue;
            }

            let name = self.decorator_name(child);
            if name.is_empty() {
                continue;
            }

            annotations.push(AnnotationInfo {
                name,
                signature: self.parser.node_text(child),
                location: self.parser.node_location(child),
                target_name: target_name.clone(),
                target_type: target_type.clone(),
                target_signature: format!("{}\n{}", self.parser.node_text(child), target_signature),
            });
        }
    }

    fn definition_header(&self, definition_node: Node<'_>) -> String {
        let end_byte = definition_node
            .child_by_field_name("body")
            .map_or_else(|| definition_node.end_byte(), |body| body.start_byte());
        self.parser
            .source_slice(definition_node.start_byte(), end_byte)
            .trim_end()
            .to_string()
    }

    fn decorator_name(&self, decorator_node: Node<'_>) -> String {
        let mut cursor = decorator_node.walk();
        for child in decorator_node.children(&mut cursor) {
            match child.kind() {
                "identifier" | "attribute" => return self.parser.node_text(child),
                "call" => {
                    return child
                        .child_by_field_name("function")
                        .map(|node| self.parser.node_text(node))
                        .unwrap_or_default();
                }
                _ => {}
            }
        }
        String::new()
    }
}
