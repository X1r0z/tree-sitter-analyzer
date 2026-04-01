use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tree_sitter::{Node, Parser, Tree};

use super::capture::{self, CallCaptureMatch};
use super::languages::python;
use crate::languages::{detect_language_engine, LanguageEngine, QueryKind};
use crate::models::{
    AnnotationInfo, CallInfo, ClassInfo, FieldInfo, FunctionInfo, FunctionParamInfo, ImportInfo,
    Location, PythonPropertyCallerInfo, PythonPropertyInfo, RefInfo, RefKey,
};

pub(crate) type PythonPropertyKey = (String, Option<String>);
pub(crate) type PythonPropertyDefinitions = HashSet<PythonPropertyKey>;
pub(crate) type PythonPropertyCallers = HashMap<String, Vec<PythonPropertyCallerInfo>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NodeId(usize);

impl NodeId {
    pub(crate) fn new(value: usize) -> Self {
        Self(value)
    }
}

impl From<usize> for NodeId {
    fn from(value: usize) -> Self {
        Self::new(value)
    }
}

impl<'a> From<Node<'a>> for NodeId {
    fn from(node: Node<'a>) -> Self {
        Self::new(node.id())
    }
}

pub(crate) struct ParseInput {
    pub(crate) file_path: PathBuf,
    pub(crate) engine: &'static dyn LanguageEngine,
    pub(crate) language: String,
    pub(crate) source: Vec<u8>,
    pub(crate) tree: Tree,
}

#[derive(Default)]
pub(crate) struct JsParseCaches {
    pub(crate) alias_events_by_function:
        HashMap<NodeId, Vec<super::languages::javascript::JsAliasEvent>>,
    pub(crate) identifier_targets: HashMap<(NodeId, usize, String), Vec<String>>,
    pub(crate) type_facts: Option<Rc<super::languages::javascript::JsTypeFacts>>,
    pub(crate) member_call_object_resolution: HashMap<NodeId, Option<String>>,
}

pub(crate) struct PythonPropertyIndexes {
    pub(crate) definitions: PythonPropertyDefinitions,
    pub(crate) callers_by_property: Option<PythonPropertyCallers>,
}

#[derive(Default)]
pub(crate) struct PythonParseCaches {
    pub(crate) property_indexes: Option<PythonPropertyIndexes>,
}

#[derive(Default)]
pub(crate) struct CommonParseCaches {
    pub(crate) function_names_by_node: HashMap<NodeId, Option<String>>,
    pub(crate) class_names_by_node: HashMap<NodeId, Option<String>>,
    pub(crate) functions_without_bodies: Option<Vec<FunctionInfo>>,
    pub(crate) classes: Option<Vec<ClassInfo>>,
    pub(crate) field_infos_by_class: HashMap<String, Vec<FieldInfo>>,
}

#[derive(Default)]
pub(crate) struct LanguageSpecificCaches {
    pub(crate) js: JsParseCaches,
    pub(crate) python: PythonParseCaches,
}

#[derive(Default)]
pub(crate) struct ParseCaches {
    pub(crate) common: CommonParseCaches,
    pub(crate) language: LanguageSpecificCaches,
}

pub(crate) struct EnclosingContext<'a> {
    pub(crate) function_name: Option<String>,
    pub(crate) class_name: Option<String>,
    pub(crate) function_node: Option<Node<'a>>,
}

pub(crate) struct ParseContext {
    pub(crate) input: ParseInput,
    pub(crate) caches: RefCell<ParseCaches>,
}

impl ParseContext {
    pub(crate) fn new(file_path: &str) -> anyhow::Result<Self> {
        let source = fs::read(file_path)?;
        Self::from_source(file_path, source)
    }

    pub(crate) fn from_source(file_path: &str, source: Vec<u8>) -> anyhow::Result<Self> {
        let path = Path::new(file_path);
        let engine = detect_language_engine(path)
            .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file_path))?;
        let mut parser = Parser::new();
        parser.set_language(&engine.ts_language())?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse: {}", file_path))?;

        Ok(Self {
            input: ParseInput {
                file_path: path.to_path_buf(),
                engine,
                language: engine.id().to_string(),
                source,
                tree,
            },
            caches: RefCell::new(ParseCaches::default()),
        })
    }

    pub(crate) fn file_path(&self) -> &Path {
        &self.input.file_path
    }

    pub(crate) fn language(&self) -> &str {
        &self.input.language
    }

    pub(crate) fn source(&self) -> &[u8] {
        &self.input.source
    }

    pub(crate) fn tree(&self) -> &Tree {
        &self.input.tree
    }

    pub(crate) fn engine(&self) -> &'static dyn crate::languages::LanguageEngine {
        self.input.engine
    }

    pub(crate) fn resolve_call_targets(
        &self,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        let Some(function_node) = self.find_enclosing_context(call_node).function_node else {
            return Vec::new();
        };

        self.engine()
            .resolve_call_targets(self, function_node, call_node, identifier_name)
    }

    pub(crate) fn node_id(&self, node: Node<'_>) -> NodeId {
        NodeId::from(node)
    }

    pub(crate) fn node_text(&self, node: Node) -> String {
        self.node_text_lossy(node).into_owned()
    }

    pub(crate) fn node_bytes<'a>(&'a self, node: Node) -> &'a [u8] {
        &self.input.source[node.start_byte()..node.end_byte()]
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
            file: self.file_path().to_string_lossy().into_owned(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        }
    }

    pub(crate) fn source_text(&self, start_byte: usize, end_byte: usize) -> String {
        String::from_utf8_lossy(&self.input.source[start_byte..end_byte]).into_owned()
    }

    pub(crate) fn find_enclosing_context<'a>(&self, node: Node<'a>) -> EnclosingContext<'a> {
        if let Some(class_name) = self.engine().enclosing_class_name(self, node) {
            return EnclosingContext {
                function_name: self.cached_function_name(node),
                class_name: Some(class_name),
                function_node: Some(node),
            };
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

            if class_name.is_none() {
                class_name = self.engine().enclosing_class_name(self, current_node);
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
        let node_id = self.node_id(node);
        if let Some(name) = self
            .caches
            .borrow()
            .common
            .function_names_by_node
            .get(&node_id)
        {
            return name.clone();
        }
        let name = self.engine().function_name(self, node);
        self.caches
            .borrow_mut()
            .common
            .function_names_by_node
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn cached_class_name(&self, node: Node) -> Option<String> {
        let node_id = self.node_id(node);
        if let Some(name) = self
            .caches
            .borrow()
            .common
            .class_names_by_node
            .get(&node_id)
        {
            return name.clone();
        }
        let name = class_name_from_node(self, node);
        self.caches
            .borrow_mut()
            .common
            .class_names_by_node
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn collect_functions(&self, include_body: bool) -> Vec<FunctionInfo> {
        if !include_body {
            if let Some(cached) = self
                .caches
                .borrow()
                .common
                .functions_without_bodies
                .as_ref()
            {
                return cached.clone();
            }
        }
        let mut func_pairs: Vec<(Node<'_>, Node<'_>)> =
            capture::collect_capture_pairs(self, QueryKind::Function, "function", "name");
        func_pairs.sort_by_key(|(f, _)| (f.start_byte(), std::cmp::Reverse(f.end_byte())));

        let mut functions = Vec::new();
        let mut seen = HashSet::new();

        for (func_node, name_node) in func_pairs {
            let function_node = self.engine().normalize_function_node(self, func_node);
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

        if !include_body {
            self.caches.borrow_mut().common.functions_without_bodies = Some(functions.clone());
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

    pub(crate) fn collect_function_params(&self, function_node: Node) -> Vec<FunctionParamInfo> {
        let target = self.engine().normalize_function_node(self, function_node);
        self.engine().function_params(self, target)
    }

    pub(crate) fn collect_classes(&self) -> Vec<ClassInfo> {
        if let Some(cached) = self.caches.borrow().common.classes.as_ref() {
            return cached.clone();
        }
        let mut methods_by_class = std::collections::HashMap::new();
        if self.language() == "go" {
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
        let mut seen = HashSet::new();
        let allow_nested_classes = matches!(self.language(), "python" | "java");

        for (class_node, name_node) in class_pairs {
            let class_node = self.engine().normalize_class_node(self, class_node);
            let name = self
                .cached_class_name(class_node)
                .unwrap_or_else(|| self.node_text(name_node));
            if name.is_empty() {
                continue;
            }
            let start = class_node.start_byte();
            let end = class_node.end_byte();
            if !seen.insert((start, end, name.clone())) {
                continue;
            }
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
            if self.language() == "go" {
                if let Some(go_methods) = methods_by_class.get(&name) {
                    method_names.extend(go_methods.iter().cloned());
                    method_names.sort_unstable();
                    method_names.dedup();
                }
            }
            let field_names = class_field_names(self, class_node);
            let super_class_names = self.engine().super_types(self, class_node);

            classes.push(ClassInfo {
                name,
                kind: class_kind(self, class_node),
                location: self.node_location(class_node),
                methods: method_names,
                fields: field_names,
                super_classes: super_class_names,
            });
        }

        self.caches.borrow_mut().common.classes = Some(classes.clone());
        classes
    }

    pub(crate) fn collect_field_infos_for_class(&self, class_name: &str) -> Vec<FieldInfo> {
        if let Some(cached) = self
            .caches
            .borrow()
            .common
            .field_infos_by_class
            .get(class_name)
        {
            return cached.clone();
        }
        if matches!(self.language(), "javascript" | "typescript" | "tsx") {
            let fields = super::languages::javascript::type_facts(self)
                .field_infos_by_class
                .get(class_name)
                .cloned()
                .unwrap_or_default();
            self.caches
                .borrow_mut()
                .common
                .field_infos_by_class
                .insert(class_name.to_string(), fields.clone());
            return fields;
        }
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
        let class_node = self.engine().normalize_class_node(self, candidates[0]);
        let fields = self.engine().class_fields(self, class_node, class_name);
        self.caches
            .borrow_mut()
            .common
            .field_infos_by_class
            .insert(class_name.to_string(), fields.clone());
        fields
    }

    pub(crate) fn collect_calls(&self) -> Vec<CallInfo> {
        let mut matched_calls: Vec<CallCaptureMatch<'_>> =
            capture::collect_call_capture_matches(self, QueryKind::Call);
        let is_js_family = matches!(self.language(), "javascript" | "typescript" | "tsx");
        if is_js_family {
            matched_calls.sort_by_key(|m| m.call.start_byte());
        }

        let mut calls = Vec::new();
        for matched in &matched_calls {
            let call_node = matched.call;
            let enclosing = self.find_enclosing_context(call_node);
            let resolved = self.engine().resolve_call(self, matched);
            let callee = resolved.callee;
            let is_method = resolved.is_method;
            let obj_name = resolved.object_name;
            let callee_function_node = resolved.callee_function_node;

            if callee.is_empty() {
                continue;
            }
            if !self
                .engine()
                .include_call(self, call_node, &callee, obj_name.as_deref())
            {
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
                let resolved = self.resolve_call_targets(call_node, &callee);
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
        self.engine().annotations(self)
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
                .input
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
        let mut stack = vec![self.tree().root_node()];
        while let Some(node) = stack.pop() {
            if node.is_named() {
                let is_ref = self.engine().is_ref_node(node);
                if is_ref {
                    if !self.engine().ref_filter(self, node) {
                        continue;
                    }
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
    match context.language() {
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
        .engine()
        .class_fields(parser, class_node, &class_name)
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
