use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use crate::languages::{detect_language, find_language, find_language_info, QueryKind};
use crate::models::*;

pub(crate) type PythonPropertyKey = (String, Option<String>);
pub(crate) type PythonPropertyDefinitions = HashSet<PythonPropertyKey>;
pub(crate) type PythonPropertyCallers = HashMap<String, Vec<PythonPropertyCallerInfo>>;

pub(crate) struct ParseContext {
    pub(crate) file_path: String,
    pub(crate) language: String,
    pub(crate) source: Vec<u8>,
    pub(crate) tree: Tree,
    pub(crate) function_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    pub(crate) class_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    pub(crate) js_alias_resolvers_by_function:
        RefCell<HashMap<usize, super::languages::javascript::JsAliasResolverState>>,
}

impl ParseContext {
    pub(crate) fn new(file_path: &str) -> anyhow::Result<Self> {
        let path = Path::new(file_path);
        let language = detect_language(path)
            .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file_path))?;
        let ts_lang = find_language(language)
            .ok_or_else(|| anyhow::anyhow!("Unsupported language: {}", language))?;
        find_language_info(language)
            .ok_or_else(|| anyhow::anyhow!("No language info for: {}", language))?;

        let source = fs::read(file_path)?;
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

    pub(crate) fn has_function_named(&self, function_name: &str, class_name: Option<&str>) -> bool {
        self.has_function_capture_named(function_name, class_name)
    }

    pub(crate) fn functions(&self, include_body: bool) -> Vec<FunctionInfo> {
        let mut func_pairs = self.query_capture_pairs(QueryKind::Function, "function", "name");
        func_pairs.sort_by_key(|(f, _)| (f.start_byte(), Reverse(f.end_byte())));

        let mut functions = Vec::new();
        let mut active_ranges: Vec<usize> = Vec::new();

        for (func_node, name_node) in func_pairs {
            let name = self.node_text(name_node);
            if name.is_empty() {
                continue;
            }
            let start = func_node.start_byte();
            let end = func_node.end_byte();
            while let Some(&active_end) = active_ranges.last() {
                if start >= active_end {
                    active_ranges.pop();
                } else {
                    break;
                }
            }
            if !active_ranges.is_empty() {
                continue;
            }
            active_ranges.push(end);
            let class_name = self.find_enclosing_class_name(func_node);
            functions.push(FunctionInfo {
                name,
                location: self.node_location(func_node),
                body: if include_body {
                    self.node_text(func_node)
                } else {
                    String::new()
                },
                class_name,
                params: self.extract_function_params(func_node),
            });
        }

        functions
    }

    pub(crate) fn imports(&self) -> Vec<ImportInfo> {
        self.query_capture_nodes(QueryKind::Import, "module")
            .into_iter()
            .map(|node| ImportInfo {
                module: self.node_text_unquoted(node).into_owned(),
                location: self.node_location(node),
            })
            .collect()
    }

    pub(crate) fn calls(&self) -> Vec<CallInfo> {
        let mut matched_calls = self.collect_call_matches(QueryKind::Call);
        let is_js_family = matches!(self.language.as_str(), "javascript" | "typescript" | "tsx");
        if is_js_family {
            matched_calls.sort_by_key(|m| m.call.start_byte());
        }

        let mut calls = Vec::new();
        for matched in &matched_calls {
            let call_node = matched.call;
            let enclosing = self.find_enclosing_context(call_node);
            let mut callee = String::new();
            let mut is_method = false;
            let mut obj_name: Option<String> = None;
            let mut callee_function_node_opt: Option<Node<'_>> = None;

            if self.language == "java" {
                let (resolved_callee, resolved_is_method, resolved_object_name) =
                    self.resolve_java_call_parts(call_node);
                callee = resolved_callee;
                is_method = resolved_is_method;
                obj_name = resolved_object_name;
            } else {
                if let Some(func_node) = call_node.child_by_field_name("function") {
                    callee_function_node_opt = Some(func_node);
                    if func_node.kind() == "identifier" {
                        callee = self.node_text(func_node);
                    } else if matches!(
                        func_node.kind(),
                        "attribute" | "member_expression" | "selector_expression"
                    ) {
                        is_method = true;
                        let (resolved_callee, resolved_object_name) =
                            self.split_attribute_parts(func_node);
                        callee = resolved_callee;
                        obj_name = resolved_object_name;
                    }
                }
                if callee.is_empty() {
                    if let Some(callee_cap) = matched.callee {
                        callee = self.node_text(callee_cap);
                    }
                    if let Some(method_cap) = matched.method {
                        callee = self.node_text(method_cap);
                        is_method = true;
                        if let Some(obj_cap) = matched.object {
                            obj_name = Some(self.node_text(obj_cap));
                        }
                    }
                }
            }

            if callee.is_empty() {
                continue;
            }

            let call_location = self.node_location(call_node);
            let mut pushed_resolved_calls = false;
            if is_js_family
                && !is_method
                && callee_function_node_opt
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
                    pushed_resolved_calls = true;
                }
            }

            if !pushed_resolved_calls {
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

    pub(crate) fn annotations(&self) -> Vec<AnnotationInfo> {
        match self.language.as_str() {
            "java" => self.extract_java_annotations(),
            "python" => self.extract_python_decorators(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn find_enclosing_function_node<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        self.find_enclosing_context(node).function_node
    }

    pub(crate) fn find_enclosing_class_name(&self, node: Node<'_>) -> Option<String> {
        self.find_enclosing_context(node).class_name
    }

    pub(crate) fn split_attribute_parts(&self, node: Node) -> (String, Option<String>) {
        let mut callee = String::new();
        let mut obj_name: Option<String> = None;

        match node.kind() {
            "attribute" => {
                if let Some(attr_node) = node.child_by_field_name("attribute") {
                    callee = self.node_text(attr_node);
                }
                if let Some(obj_node) = node.child_by_field_name("object") {
                    obj_name = Some(self.node_text(obj_node));
                }
            }
            "member_expression" => {
                if let Some(prop_node) = node.child_by_field_name("property") {
                    callee = self.node_text(prop_node);
                }
                if let Some(obj_node) = node.child_by_field_name("object") {
                    obj_name = Some(self.node_text(obj_node));
                }
            }
            "selector_expression" => {
                if let Some(field_node) = node.child_by_field_name("field") {
                    callee = self.node_text(field_node);
                }
                if let Some(operand_node) = node.child_by_field_name("operand") {
                    obj_name = Some(self.node_text(operand_node));
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
                        ids.push(self.node_text(child));
                    } else if matches!(
                        child.kind(),
                        "attribute" | "member_expression" | "selector_expression"
                    ) {
                        obj_name = Some(self.node_text(child));
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

    pub(crate) fn extract_function_params(&self, function_node: Node) -> Vec<FunctionParamInfo> {
        let target = self.unwrap_callable_node(function_node);
        match self.language.as_str() {
            "python" => self.extract_python_function_params(target),
            "javascript" | "typescript" | "tsx" => self.extract_js_function_params(target),
            "java" => self.extract_java_function_params(target),
            "go" => self.extract_go_function_params(target),
            _ => Vec::new(),
        }
    }

    fn unwrap_callable_node<'a>(&self, node: Node<'a>) -> Node<'a> {
        if node.kind() == "decorated_definition" {
            if let Some(definition) = node.child_by_field_name("definition") {
                return definition;
            }
        }
        node
    }
}
