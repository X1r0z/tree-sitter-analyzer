use std::cell::Ref;
use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::super::capture::CallCaptureMatch;
use super::super::{
    EnclosingContext, ParseContext, PythonPropertyCallers, PythonPropertyDefinitions,
    PythonPropertyIndexes,
};
use crate::languages::{LanguageEngine, ResolvedCall};
use crate::models::{
    AnnotationInfo, FieldInfo, FunctionParamInfo, Location, PythonPropertyCallerInfo,
    PythonPropertyInfo,
};
use crate::parser::call_targets::{
    matches_property_target as call_matches_property_target, type_matches_class,
};
use crate::traversal::collect_reachable_bfs;
use crate::utils::select_most_specific_by_line;

type PythonPropertyCallerKey = (String, String, Option<String>, Option<String>, usize, usize);

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

pub(crate) struct PythonPropertyAnalyzer<'a> {
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

    fn super_types(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
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
            if let Some(info) = self.build_parameter_info(param) {
                params.push(info);
            }
        }
        params
    }

    fn build_parameter_info(&self, param: Node<'_>) -> Option<FunctionParamInfo> {
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
            .source_text(definition_node.start_byte(), end_byte)
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

impl<'a> PythonPropertyAnalyzer<'a> {
    pub(crate) fn new(parser: &'a ParseContext) -> Self {
        Self { parser }
    }

    pub(crate) fn collect_infos(&self) -> Vec<PythonPropertyInfo> {
        self.with_cached_definitions(|properties| {
            let mut values: Vec<_> = properties
                .iter()
                .cloned()
                .map(|(name, class_name)| PythonPropertyInfo { name, class_name })
                .collect();
            values.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then_with(|| left.class_name.cmp(&right.class_name))
            });
            values
        })
    }

    pub(crate) fn collect_callers(
        &self,
        property_name: Option<&str>,
    ) -> Vec<PythonPropertyCallerInfo> {
        self.with_cached_callers(|callers| {
            let mut values = Vec::new();

            match property_name {
                Some(property_name) => {
                    values.extend(callers.get(property_name).cloned().unwrap_or_default());
                }
                None => {
                    for entries in callers.values() {
                        values.extend(entries.iter().cloned());
                    }
                }
            }

            values.sort_by(|left, right| {
                left.property_name
                    .cmp(&right.property_name)
                    .then_with(|| left.caller.cmp(&right.caller))
                    .then_with(|| left.location.start_line.cmp(&right.location.start_line))
            });
            values
        })
    }

    fn ensure_definitions_cached(&self) {
        if self.parser.language() != "python" {
            return;
        }
        if self
            .parser
            .caches
            .borrow()
            .language
            .python
            .property_indexes
            .is_some()
        {
            return;
        }
        let definitions = self.collect_definitions();
        self.parser
            .caches
            .borrow_mut()
            .language
            .python
            .property_indexes = Some(PythonPropertyIndexes {
            definitions,
            callers_by_property: None,
        });
    }

    fn ensure_callers_cached(&self) {
        if self.parser.language() != "python" {
            return;
        }
        self.ensure_definitions_cached();

        let should_build = self
            .parser
            .caches
            .borrow()
            .language
            .python
            .property_indexes
            .as_ref()
            .is_some_and(|indexes| indexes.callers_by_property.is_none());
        if !should_build {
            return;
        }

        let definitions = {
            let caches = self.parser.caches.borrow();
            caches
                .language
                .python
                .property_indexes
                .as_ref()
                .expect("python cache")
                .definitions
                .clone()
        };
        let candidate_callers = self.collect_candidate_callers();
        let callers_by_property = self.filter_callers(&definitions, &candidate_callers);
        if let Some(indexes) = self
            .parser
            .caches
            .borrow_mut()
            .language
            .python
            .property_indexes
            .as_mut()
        {
            indexes.callers_by_property = Some(callers_by_property);
        }
    }

    fn with_cached_definitions<R>(&self, f: impl FnOnce(&PythonPropertyDefinitions) -> R) -> R {
        if self.parser.language() != "python" {
            return f(&HashSet::new());
        }
        self.ensure_definitions_cached();
        let caches = self.parser.caches.borrow();
        let definitions = Ref::map(caches, |caches| {
            &caches
                .language
                .python
                .property_indexes
                .as_ref()
                .expect("python cache")
                .definitions
        });
        f(&definitions)
    }

    fn with_cached_callers<R>(&self, f: impl FnOnce(&PythonPropertyCallers) -> R) -> R {
        if self.parser.language() != "python" {
            return f(&HashMap::new());
        }
        self.ensure_callers_cached();
        let caches = self.parser.caches.borrow();
        let callers = Ref::map(caches, |caches| {
            caches
                .language
                .python
                .property_indexes
                .as_ref()
                .expect("python cache")
                .callers_by_property
                .as_ref()
                .expect("python callers cache")
        });
        f(&callers)
    }

    fn collect_definitions(&self) -> PythonPropertyDefinitions {
        if self.parser.language() != "python" {
            return HashSet::new();
        }

        let mut properties = HashSet::new();
        let mut stack = vec![self.parser.tree().root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "decorated_definition" {
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
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "decorator"
                        && self.parser.node_trimmed_text_eq(child, "@property")
                    {
                        is_property = true;
                        break;
                    }
                }

                if is_property {
                    properties.insert((
                        self.parser.node_text(name_node),
                        self.parser
                            .find_enclosing_context(definition_node)
                            .class_name,
                    ));
                }
            }

            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }

        properties
    }

    fn collect_candidate_callers(&self) -> PythonPropertyCallers {
        if self.parser.language() != "python" {
            return HashMap::new();
        }

        let module_binding_types = self.collect_module_binding_types();
        let mut callers_by_property: HashMap<String, Vec<PythonPropertyCallerInfo>> =
            HashMap::new();
        let mut seen_callers: HashSet<PythonPropertyCallerKey> = HashSet::new();
        let mut stack = vec![self.parser.tree().root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "attribute" {
                if !Self::is_load_like_property_access(node) {
                    continue;
                }
                let parts = AttributeParts::from_attribute(self.parser, node);
                if parts.name.is_empty() {
                    continue;
                }
                let enclosing = self.parser.find_enclosing_context(node);
                let caller = enclosing
                    .function_name
                    .unwrap_or_else(|| "<module>".to_string());
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;
                let object_type = if caller == "<module>" {
                    parts
                        .object
                        .as_deref()
                        .and_then(|name| module_binding_types.get(name))
                        .cloned()
                } else {
                    None
                };
                let seen_key = (
                    parts.name.clone(),
                    caller.clone(),
                    enclosing.class_name.clone(),
                    parts.object.clone(),
                    start_line,
                    end_line,
                );
                if seen_callers.insert(seen_key) {
                    callers_by_property
                        .entry(parts.name.clone())
                        .or_default()
                        .push(PythonPropertyCallerInfo {
                            location: Location {
                                file: self.parser.file_path().to_string_lossy().into_owned(),
                                start_line,
                                end_line,
                            },
                            property_name: parts.name.clone(),
                            caller,
                            caller_class_name: enclosing.class_name.clone(),
                            object_name: parts.object,
                            object_type,
                        });
                }
            }

            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }

        callers_by_property
    }

    fn is_load_like_property_access(node: Node<'_>) -> bool {
        let mut current = node;
        while let Some(parent) = current.parent() {
            match parent.kind() {
                "assignment" | "augmented_assignment" => {
                    if parent.child_by_field_name("left").is_some_and(|left| {
                        left.start_byte() <= node.start_byte() && node.end_byte() <= left.end_byte()
                    }) {
                        return false;
                    }
                }
                _ => {}
            }
            current = parent;
        }
        true
    }

    fn collect_module_binding_types(&self) -> HashMap<String, String> {
        let mut bindings = HashMap::new();
        let root = self.parser.tree().root_node();

        let mut cursor = root.walk();
        for node in root.named_children(&mut cursor) {
            let mut assignments = Vec::new();
            match node.kind() {
                "expression_statement" => {
                    let mut cursor = node.walk();
                    assignments.extend(
                        node.named_children(&mut cursor)
                            .filter(|child| child.kind() == "assignment"),
                    );
                }
                "assignment" => assignments.push(node),
                _ => {}
            }

            for assignment in assignments {
                let Some(left) = assignment.child_by_field_name("left") else {
                    continue;
                };
                if left.kind() != "identifier" {
                    continue;
                }
                let name = self.parser.node_text(left);
                if name.is_empty() {
                    continue;
                }

                if let Some(type_node) = assignment.child_by_field_name("type") {
                    let class_name = self
                        .parser
                        .node_text(type_node)
                        .rsplit('.')
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    if !class_name.is_empty() {
                        bindings.insert(name, class_name);
                        continue;
                    }
                }

                let Some(right) = assignment.child_by_field_name("right") else {
                    bindings.remove(&name);
                    continue;
                };
                if let Some(class_name) = self.infer_module_binding_type(right) {
                    bindings.insert(name, class_name);
                } else {
                    bindings.remove(&name);
                }
            }
        }

        bindings
    }

    fn infer_module_binding_type(&self, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "call" => node
                .child_by_field_name("function")
                .and_then(|function| self.infer_module_binding_type(function)),
            "identifier" | "attribute" => {
                let class_name = self
                    .parser
                    .node_text(node)
                    .rsplit('.')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                (!class_name.is_empty()).then_some(class_name)
            }
            _ => None,
        }
    }

    fn filter_callers(
        &self,
        property_definitions: &PythonPropertyDefinitions,
        callers: &PythonPropertyCallers,
    ) -> PythonPropertyCallers {
        let property_definitions: Vec<_> = property_definitions
            .iter()
            .cloned()
            .map(|(name, class_name)| PythonPropertyInfo { name, class_name })
            .collect();
        let parents_by_class = self
            .parser
            .collect_classes()
            .into_iter()
            .map(|class| (class.name, class.super_classes))
            .collect::<HashMap<_, _>>();
        let mut functions = self.parser.collect_functions(false);
        functions
            .sort_by_key(|function| (function.location.start_line, function.location.end_line));

        let mut filtered = HashMap::new();
        for (property_name, entries) in callers {
            let matched: Vec<_> = entries
                .iter()
                .filter(|caller| {
                    self.matches_known_property(
                        caller,
                        &property_definitions,
                        &parents_by_class,
                        &functions,
                    )
                })
                .cloned()
                .collect();
            if !matched.is_empty() {
                filtered.insert(property_name.clone(), matched);
            }
        }
        filtered
    }

    fn matches_known_property(
        &self,
        caller: &PythonPropertyCallerInfo,
        property_definitions: &[PythonPropertyInfo],
        parents_by_class: &HashMap<String, Vec<String>>,
        functions: &[crate::models::FunctionInfo],
    ) -> bool {
        let candidates: Vec<_> = property_definitions
            .iter()
            .filter(|property| property.name == caller.property_name)
            .collect();
        if candidates.is_empty() {
            return false;
        }

        if caller.caller == "<module>" {
            return candidates.iter().any(|property| {
                property.class_name.as_deref().is_some_and(|class_name| {
                    caller.object_name.as_deref() != Some(class_name)
                        && type_matches_class(caller.object_type.as_deref(), class_name)
                })
            });
        }

        let caller_params =
            select_most_specific_by_line(functions, caller.location.start_line, |function| {
                (function.location.start_line, function.location.end_line)
            })
            .filter(|function| {
                function.name == caller.caller
                    && function.class_name.as_deref() == caller.caller_class_name.as_deref()
            })
            .map(|function| function.params.clone())
            .unwrap_or_default();

        let caller_fields = caller
            .caller_class_name
            .as_deref()
            .map(|class_name| self.parser.collect_field_infos_for_class(class_name))
            .unwrap_or_default();

        candidates.iter().any(|property| {
            let Some(target_class_name) = property.class_name.as_deref() else {
                return false;
            };

            if matches!(caller.object_name.as_deref(), Some("self" | "cls")) {
                caller
                    .caller_class_name
                    .as_deref()
                    .is_some_and(|class_name| {
                        class_name == target_class_name
                            || collect_reachable_bfs(
                                [class_name.to_string()],
                                [class_name.to_string()],
                                |current| {
                                    parents_by_class.get(current).cloned().unwrap_or_default()
                                },
                                Clone::clone,
                                Clone::clone,
                            )
                            .into_iter()
                            .any(|ancestor| ancestor == target_class_name)
                    })
            } else {
                call_matches_property_target(
                    caller.caller_class_name.as_deref(),
                    caller.object_name.as_deref(),
                    target_class_name,
                    |attr_name, expected_class_name| {
                        caller_fields.iter().any(|field| {
                            field.name == attr_name
                                && type_matches_class(
                                    field.field_type.as_deref(),
                                    expected_class_name,
                                )
                        })
                    },
                    |attr_name, expected_class_name| {
                        caller_params.iter().any(|param| {
                            param.name == attr_name
                                && type_matches_class(
                                    param.param_type.as_deref(),
                                    expected_class_name,
                                )
                        })
                    },
                )
            }
        })
    }
}
