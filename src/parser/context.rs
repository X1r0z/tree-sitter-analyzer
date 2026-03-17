use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use super::capture::{self, CallCaptureMatch};
use super::languages::{go, java, javascript, python};
use crate::languages::{detect_language, find_language, find_language_info, QueryKind};
use crate::models::{
    AnnotationInfo, CallInfo, ClassInfo, FieldInfo, FunctionInfo, FunctionParamInfo, ImportInfo,
    Location, PythonPropertyCallerInfo, PythonPropertyInfo, RefInfo, RefKey,
};

pub(crate) type PythonPropertyKey = (String, Option<String>);
pub(crate) type PythonPropertyDefinitions = HashSet<PythonPropertyKey>;
pub(crate) type PythonPropertyCallers = HashMap<String, Vec<PythonPropertyCallerInfo>>;

pub(crate) struct EnclosingContext<'a> {
    pub(crate) function_name: Option<String>,
    pub(crate) class_name: Option<String>,
    pub(crate) function_node: Option<Node<'a>>,
}

pub(crate) struct ParseContext {
    pub(crate) file_path: String,
    pub(crate) language: String,
    pub(crate) source: Vec<u8>,
    pub(crate) tree: Tree,
    pub(crate) function_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    pub(crate) class_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    pub(crate) js_alias_resolvers_by_function:
        RefCell<HashMap<usize, super::languages::javascript::JsAliasResolverState>>,
    pub(crate) python_property_indexes:
        RefCell<Option<(PythonPropertyDefinitions, PythonPropertyCallers)>>,
}

impl ParseContext {
    pub(crate) fn new(file_path: &str) -> anyhow::Result<Self> {
        let source = fs::read(file_path)?;
        Self::from_source(file_path, source)
    }

    pub(crate) fn from_source(file_path: &str, source: Vec<u8>) -> anyhow::Result<Self> {
        let path = Path::new(file_path);
        let language = detect_language(path)
            .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file_path))?;
        let ts_lang = find_language(language)
            .ok_or_else(|| anyhow::anyhow!("Unsupported language: {}", language))?;
        find_language_info(language)
            .ok_or_else(|| anyhow::anyhow!("No language info for: {}", language))?;
        let mut parser = Parser::new();
        parser.set_language(&ts_lang)?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse: {}", file_path))?;

        Ok(Self {
            file_path: file_path.to_string(),
            language: language.to_string(),
            source,
            tree,
            function_names_by_node: RefCell::new(HashMap::new()),
            class_names_by_node: RefCell::new(HashMap::new()),
            js_alias_resolvers_by_function: RefCell::new(HashMap::new()),
            python_property_indexes: RefCell::new(None),
        })
    }

    pub(crate) fn node_text(&self, node: Node) -> String {
        self.node_text_lossy(node).into_owned()
    }

    pub(crate) fn node_bytes<'a>(&'a self, node: Node) -> &'a [u8] {
        &self.source[node.start_byte()..node.end_byte()]
    }

    pub(crate) fn node_text_lossy<'a>(&'a self, node: Node) -> Cow<'a, str> {
        String::from_utf8_lossy(self.node_bytes(node))
    }

    pub(crate) fn node_text_eq(&self, node: Node, text: &str) -> bool {
        self.node_bytes(node) == text.as_bytes()
    }

    pub(crate) fn node_trimmed_text_eq(&self, node: Node, text: &str) -> bool {
        self.node_text_lossy(node).trim() == text
    }

    pub(crate) fn node_text_unquoted(&self, node: Node) -> Cow<'_, str> {
        let bytes = self.node_bytes(node);
        if bytes.len() >= 2 {
            let first = bytes[0];
            let last = bytes[bytes.len() - 1];
            if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                return String::from_utf8_lossy(&bytes[1..bytes.len() - 1]);
            }
        }
        self.node_text_lossy(node)
    }

    pub(crate) fn node_location(&self, node: Node) -> Location {
        Location {
            file: self.file_path.clone(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        }
    }

    pub(crate) fn source_text(&self, start_byte: usize, end_byte: usize) -> String {
        String::from_utf8_lossy(&self.source[start_byte..end_byte]).into_owned()
    }

    pub(crate) fn find_enclosing_context<'a>(&self, node: Node<'a>) -> EnclosingContext<'a> {
        if self.language == "go" && node.kind() == "method_declaration" {
            if let Some(class_name) = go::receiver_type_name(self, node) {
                return EnclosingContext {
                    function_name: self.cached_function_name(node),
                    class_name: Some(class_name),
                    function_node: Some(node),
                };
            }
        }

        let mut current = node.parent();
        let mut function_name: Option<String> = None;
        let mut class_name: Option<String> = None;
        let mut function_node: Option<Node<'a>> = None;

        while let Some(current_node) = current {
            if function_name.is_none() && is_function_like(current_node.kind()) {
                function_name = self.cached_function_name(current_node);
                function_node = Some(current_node);
            }

            if class_name.is_none()
                && self.language == "go"
                && current_node.kind() == "method_declaration"
            {
                class_name = go::receiver_type_name(self, current_node);
            }

            if class_name.is_none()
                && matches!(
                    current_node.kind(),
                    "class_definition"
                        | "class_declaration"
                        | "class_body"
                        | "interface_declaration"
                        | "enum_declaration"
                        | "record_declaration"
                        | "annotation_type_declaration"
                )
            {
                class_name = self.cached_class_name(current_node);
                current = current_node.parent();
                continue;
            }

            current = current_node.parent();
        }

        EnclosingContext {
            function_name,
            class_name,
            function_node,
        }
    }

    pub(crate) fn cached_function_name(&self, node: Node) -> Option<String> {
        let node_id = node.id();
        if let Some(name) = self.function_names_by_node.borrow().get(&node_id) {
            return name.clone();
        }
        let name = match self.language.as_str() {
            "python" => python::function_name_from_node(self, node),
            "javascript" | "typescript" | "tsx" => javascript::function_name_from_node(self, node),
            "java" => java::function_name_from_node(self, node),
            "go" => go::function_name_from_node(self, node),
            _ => None,
        };
        self.function_names_by_node
            .borrow_mut()
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn cached_class_name(&self, node: Node) -> Option<String> {
        let node_id = node.id();
        if let Some(name) = self.class_names_by_node.borrow().get(&node_id) {
            return name.clone();
        }
        let name = class_name_from_node(self, node);
        self.class_names_by_node
            .borrow_mut()
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn collect_functions(&self, include_body: bool) -> Vec<FunctionInfo> {
        let mut func_pairs: Vec<(Node<'_>, Node<'_>)> =
            capture::collect_capture_pairs(self, QueryKind::Function, "function", "name");
        func_pairs.sort_by_key(|(f, _)| (f.start_byte(), std::cmp::Reverse(f.end_byte())));

        let mut functions = Vec::new();
        let mut seen = HashSet::new();

        for (func_node, name_node) in func_pairs {
            let function_node = match self.language.as_str() {
                "python" => python::unwrap_callable_node(func_node),
                _ => func_node,
            };
            let name = self
                .cached_function_name(function_node)
                .unwrap_or_else(|| self.node_text(name_node));
            if name.is_empty() {
                continue;
            }
            let class_name = self.find_enclosing_context(function_node).class_name;
            let key = (
                function_node.start_byte(),
                function_node.end_byte(),
                name.clone(),
                class_name.clone(),
            );
            if !seen.insert(key) {
                continue;
            }
            functions.push(FunctionInfo {
                name,
                location: self.node_location(function_node),
                body: if include_body {
                    self.node_text(function_node)
                } else {
                    String::new()
                },
                class_name,
                params: self.collect_function_params(function_node),
            });
        }

        functions
    }

    pub(crate) fn collect_python_properties(&self) -> Vec<PythonPropertyInfo> {
        python::collect_property_infos(self)
    }

    pub(crate) fn collect_python_property_callers(
        &self,
        property_name: Option<&str>,
    ) -> Vec<PythonPropertyCallerInfo> {
        python::collect_property_callers(self, property_name)
    }

    pub(crate) fn has_python_property_definition(
        &self,
        property_name: &str,
        class_name: Option<&str>,
    ) -> bool {
        python::has_property_definition(self, property_name, class_name)
    }

    pub(crate) fn collect_function_params(&self, function_node: Node) -> Vec<FunctionParamInfo> {
        let target = match self.language.as_str() {
            "python" => python::unwrap_callable_node(function_node),
            _ => function_node,
        };
        match self.language.as_str() {
            "python" => python::function_params(self, target),
            "javascript" | "typescript" | "tsx" => javascript::function_params(self, target),
            "java" => java::function_params(self, target),
            "go" => go::function_params(self, target),
            _ => Vec::new(),
        }
    }

    pub(crate) fn collect_classes(&self) -> Vec<ClassInfo> {
        let mut methods_by_class = std::collections::HashMap::new();
        if self.language == "go" {
            for function in self.collect_functions(false) {
                if let Some(class_name) = function.class_name {
                    methods_by_class
                        .entry(class_name)
                        .or_insert_with(Vec::new)
                        .push(function.name);
                }
            }
            for methods in methods_by_class.values_mut() {
                methods.sort_unstable();
                methods.dedup();
            }
        }

        let mut class_pairs: Vec<(Node<'_>, Node<'_>)> =
            capture::collect_capture_pairs(self, QueryKind::Class, "class", "name");
        class_pairs.sort_by_key(|(class_node, _)| {
            (
                class_node.start_byte(),
                std::cmp::Reverse(class_node.end_byte()),
            )
        });

        let mut classes = Vec::new();
        let mut active_ranges: Vec<usize> = Vec::new();
        let allow_nested_classes = matches!(self.language.as_str(), "python" | "java");

        for (class_node, name_node) in class_pairs {
            let name = self.node_text(name_node);
            if name.is_empty() {
                continue;
            }
            let start = class_node.start_byte();
            let end = class_node.end_byte();
            while let Some(&active_end) = active_ranges.last() {
                if start >= active_end {
                    active_ranges.pop();
                } else {
                    break;
                }
            }
            let is_nested = !active_ranges.is_empty();
            if is_nested && !allow_nested_classes {
                continue;
            }
            active_ranges.push(end);

            let mut method_names = class_method_names(self, class_node);
            if self.language == "go" {
                if let Some(go_methods) = methods_by_class.get(&name) {
                    method_names.extend(go_methods.iter().cloned());
                    method_names.sort_unstable();
                    method_names.dedup();
                }
            }
            let field_names = class_field_names(self, class_node);
            let super_class_names = self.collect_super_class_names(class_node);

            classes.push(ClassInfo {
                name,
                kind: class_kind(self, class_node),
                location: self.node_location(class_node),
                methods: method_names,
                fields: field_names,
                super_classes: super_class_names,
            });
        }

        classes
    }

    pub(crate) fn collect_class_field_infos(
        &self,
        class_node: Node,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        match self.language.as_str() {
            "python" => python::field_infos(self, class_node, class_name),
            "javascript" | "typescript" | "tsx" => {
                javascript::field_infos(self, class_node, class_name)
            }
            "java" => java::field_infos(self, class_node, class_name),
            "go" => go::field_infos(self, class_node, class_name),
            _ => Vec::new(),
        }
    }

    pub(crate) fn collect_super_class_names(&self, class_node: Node) -> Vec<String> {
        match self.language.as_str() {
            "python" => python::super_class_names(self, class_node),
            "javascript" | "typescript" | "tsx" => javascript::super_class_names(self, class_node),
            "java" => java::super_class_names(self, class_node),
            "go" => go::embedded_type_names(self, class_node),
            _ => Vec::new(),
        }
    }

    pub(crate) fn collect_field_infos_for_class(&self, class_name: &str) -> Vec<FieldInfo> {
        let mut candidates: Vec<Node<'_>> = Vec::new();
        for (class_node, name_node) in
            capture::collect_capture_pairs(self, QueryKind::Class, "class", "name")
        {
            if self.node_text_eq(name_node, class_name) {
                candidates.push(class_node);
            }
        }
        if candidates.is_empty() {
            return Vec::new();
        }

        candidates.sort_by_key(|node| {
            let size = node.end_byte() - node.start_byte();
            (size, node.start_byte())
        });
        self.collect_class_field_infos(candidates[0], class_name)
    }

    pub(crate) fn collect_calls(&self) -> Vec<CallInfo> {
        let mut matched_calls: Vec<CallCaptureMatch<'_>> =
            capture::collect_call_capture_matches(self, QueryKind::Call);
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
                    java::resolve_call_parts(self, call_node);
                (callee_name, resolved_is_method, object_name, None)
            } else {
                match self.language.as_str() {
                    "python" => python::resolve_call_parts(self, matched),
                    "javascript" | "typescript" | "tsx" => {
                        javascript::resolve_call_parts(self, matched)
                    }
                    "go" => go::resolve_call_parts(self, matched),
                    _ => (String::new(), false, None, None),
                }
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
                let resolved = self.resolve_call_targets_for_identifier(call_node, &callee);
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

    pub(crate) fn collect_imports(&self) -> Vec<ImportInfo> {
        capture::collect_import_capture_matches(self, QueryKind::Import)
            .into_iter()
            .map(|import_match| ImportInfo {
                module: self.node_text_unquoted(import_match.module).into_owned(),
                location: self.node_location(import_match.import.unwrap_or(import_match.module)),
            })
            .collect()
    }

    pub(crate) fn collect_annotations(&self) -> Vec<AnnotationInfo> {
        match self.language.as_str() {
            "java" => java::extract_annotations(self),
            "python" => python::extract_decorators(self),
            _ => Vec::new(),
        }
    }

    pub(crate) fn collect_refs(&self) -> Vec<RefInfo> {
        self.scan_refs(|node| {
            let name = self.node_text(node);
            (!name.is_empty()).then(|| RefInfo {
                name,
                node_type: node.kind().to_string(),
                location: self.node_location(node),
                start_column: node.start_position().column,
                end_column: node.end_position().column,
                context: String::new(),
            })
        })
    }

    pub(crate) fn find_refs(&self, name: &str, with_context: bool) -> Vec<RefInfo> {
        let name_bytes = name.as_bytes();
        if name_bytes.is_empty()
            || !self
                .source
                .windows(name_bytes.len())
                .any(|window| window == name_bytes)
        {
            return Vec::new();
        }

        self.scan_refs(|node| {
            (self.node_bytes(node) == name_bytes).then(|| RefInfo {
                name: name.to_string(),
                node_type: node.kind().to_string(),
                location: self.node_location(node),
                start_column: node.start_position().column,
                end_column: node.end_position().column,
                context: if with_context {
                    node.parent()
                        .map(|parent| self.node_text(parent))
                        .unwrap_or_default()
                } else {
                    String::new()
                },
            })
        })
    }

    pub(crate) fn hydrate_refs(&self, candidates: &[RefInfo]) -> Vec<RefInfo> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let expected: std::collections::HashSet<RefKey> =
            candidates.iter().map(RefKey::from).collect();
        self.scan_refs(|node| {
            let reference = RefInfo {
                name: self.node_text(node),
                node_type: node.kind().to_string(),
                location: self.node_location(node),
                start_column: node.start_position().column,
                end_column: node.end_position().column,
                context: String::new(),
            };
            expected
                .contains(&RefKey::from(&reference))
                .then(|| RefInfo {
                    context: node
                        .parent()
                        .map(|parent| self.node_text(parent))
                        .unwrap_or_default(),
                    ..reference
                })
        })
    }

    fn scan_refs(&self, mut map_node: impl FnMut(Node<'_>) -> Option<RefInfo>) -> Vec<RefInfo> {
        let mut refs = Vec::new();
        let mut stack = vec![self.tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.is_named() {
                let is_ref = match self.language.as_str() {
                    "python" => python::is_ref_node(node),
                    "javascript" | "typescript" | "tsx" => javascript::is_ref_node(node),
                    "java" => java::is_ref_node(node),
                    "go" => go::is_ref_node(node),
                    _ => false,
                };
                if is_ref {
                    if let Some(reference) = map_node(node) {
                        refs.push(reference);
                    }
                }
                for i in (0..node.named_child_count()).rev() {
                    if let Some(child) = node.named_child(i as u32) {
                        stack.push(child);
                    }
                }
            }
        }
        refs
    }
}

fn is_function_like(node_kind: &str) -> bool {
    matches!(
        node_kind,
        "function_definition"
            | "async_function_definition"
            | "function_declaration"
            | "method_definition"
            | "arrow_function"
            | "method_declaration"
            | "constructor_declaration"
            | "function_expression"
            | "func_literal"
    )
}

fn class_name_from_node(context: &ParseContext, node: Node<'_>) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let class_name = context.node_text(name_node);
        if !class_name.is_empty() {
            return Some(class_name);
        }
    }
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if matches!(child.kind(), "identifier" | "type_identifier" | "name") {
            let class_name = context.node_text(child);
            if !class_name.is_empty() {
                return Some(class_name);
            }
        }
    }
    None
}

fn class_kind(context: &ParseContext, node: Node<'_>) -> String {
    match context.language.as_str() {
        "python" | "javascript" => "class".to_string(),
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
            .map(|type_node| match type_node.kind() {
                "interface_type" => "interface",
                "struct_type" => "struct",
                _ => "struct",
            })
            .unwrap_or("struct")
            .to_string(),
        _ => "class".to_string(),
    }
}

pub(crate) fn collect_field_infos_from_declarations(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
    include_embedded_type_names: bool,
) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![class_node];

    while let Some(node) = stack.pop() {
        if node.id() != class_node.id()
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
                .map(|child| parser.node_text(child))
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
                .map(|child| parser.node_text(child));

            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier"
                ) {
                    names.push(parser.node_text(child));
                } else if child.kind() == "variable_declarator" {
                    for j in 0..child.child_count() {
                        let sub = child.child(j as u32).unwrap();
                        if sub.kind() == "identifier" {
                            names.push(parser.node_text(sub));
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
                    field_type = Some(parser.node_text(child));
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
                        location: parser.node_location(node),
                        field_type: field_type.clone(),
                        class_name: Some(class_name.to_string()),
                    });
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

    fields
}

fn class_method_names(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
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
            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier" | "name"
                ) {
                    methods.push(parser.node_text(child));
                    break;
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
    methods
}

fn class_field_names(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
    let class_name = class_node
        .child_by_field_name("name")
        .map(|child| parser.node_text(child))
        .unwrap_or_default();
    parser
        .collect_class_field_infos(class_node, &class_name)
        .into_iter()
        .map(|field| field.name)
        .collect()
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
