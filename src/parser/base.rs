use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor, Tree};

use crate::languages::{
    detect_language, find_compiled_query, find_language, find_language_info, LanguageInfo,
};
use crate::models::*;

pub(crate) struct BaseParser {
    pub(crate) file_path: String,
    pub(crate) language: String,
    pub(crate) source: Vec<u8>,
    pub(crate) tree: Tree,
    pub(crate) language_info: &'static LanguageInfo,
    function_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    class_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    pub(super) js_alias_resolvers_by_function:
        RefCell<HashMap<usize, super::javascript::JsAliasResolverState>>,
}

pub(crate) struct CallQueryMatch<'a> {
    pub(crate) call: Node<'a>,
    pub(crate) callee: Option<Node<'a>>,
    pub(crate) method: Option<Node<'a>>,
    pub(crate) object: Option<Node<'a>>,
}

pub(crate) type PythonPropertyKey = (String, Option<String>);
pub(crate) type PythonPropertyDefinitions = HashSet<PythonPropertyKey>;
pub(crate) type PythonPropertyCallers = HashMap<String, Vec<(String, usize)>>;

#[allow(dead_code)]
impl BaseParser {
    pub(crate) fn new(file_path: &str) -> anyhow::Result<Self> {
        let path = Path::new(file_path);
        let language = detect_language(path)
            .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file_path))?;
        let ts_lang = find_language(language)
            .ok_or_else(|| anyhow::anyhow!("Unsupported language: {}", language))?;
        let language_info = find_language_info(language)
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
            language_info,
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

    pub(crate) fn node_eq_str(&self, node: Node, text: &str) -> bool {
        self.node_bytes(node) == text.as_bytes()
    }

    pub(crate) fn node_trimmed_eq_str(&self, node: Node, text: &str) -> bool {
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

    pub(crate) fn extract_python_definition_header(&self, definition_node: Node) -> String {
        let end_byte = definition_node
            .child_by_field_name("body")
            .map(|body| body.start_byte())
            .unwrap_or_else(|| definition_node.end_byte());
        self.source_text(definition_node.start_byte(), end_byte)
            .trim_end()
            .to_string()
    }

    pub(crate) fn extract_java_signature(&self, declaration_node: Node) -> String {
        let end_byte = declaration_node
            .child_by_field_name("body")
            .map(|body| body.start_byte())
            .unwrap_or_else(|| declaration_node.end_byte());
        let start_byte = self.java_signature_start_byte(declaration_node);
        self.source_text(start_byte, end_byte)
            .trim_end()
            .trim_end_matches(';')
            .trim_end()
            .to_string()
    }

    pub(crate) fn extract_java_class_header_line(&self, declaration_node: Node) -> String {
        let signature = self.extract_java_signature(declaration_node);
        signature
            .lines()
            .next()
            .unwrap_or("")
            .trim_end()
            .to_string()
    }

    fn java_signature_start_byte(&self, declaration_node: Node) -> usize {
        for i in 0..declaration_node.child_count() {
            let Some(child) = declaration_node.child(i as u32) else {
                continue;
            };
            match child.kind() {
                "marker_annotation" | "annotation" => continue,
                "modifiers" => {
                    let mut cursor = child.walk();
                    for modifier_child in child.children(&mut cursor) {
                        if matches!(modifier_child.kind(), "marker_annotation" | "annotation") {
                            continue;
                        }
                        return modifier_child.start_byte();
                    }
                    continue;
                }
                _ => return child.start_byte(),
            }
        }

        declaration_node.start_byte()
    }

    fn with_query<T>(&self, query_str: &str, f: impl FnOnce(&Query) -> T) -> Option<T> {
        if let Some(query) = find_compiled_query(&self.language, query_str) {
            return Some(f(query));
        }
        let ts_lang = find_language(&self.language)?;
        let query = match Query::new(&ts_lang, query_str) {
            Ok(q) => q,
            Err(_) => return None,
        };
        Some(f(&query))
    }

    pub(crate) fn query_capture_nodes(&self, query_str: &str, capture_name: &str) -> Vec<Node<'_>> {
        self.with_query(query_str, |query| {
            let Some(capture_index) = query
                .capture_names()
                .iter()
                .position(|name| *name == capture_name)
                .map(|idx| idx as u32)
            else {
                return Vec::new();
            };

            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(query, self.tree.root_node(), self.source.as_slice());
            let mut nodes = Vec::new();
            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index == capture_index {
                        nodes.push(cap.node);
                    }
                }
            }
            nodes
        })
        .unwrap_or_default()
    }

    pub(crate) fn query_capture_pairs(
        &self,
        query_str: &str,
        first_capture: &str,
        second_capture: &str,
    ) -> Vec<(Node<'_>, Node<'_>)> {
        self.with_query(query_str, |query| {
            let capture_names = query.capture_names();
            let Some(first_index) = capture_names
                .iter()
                .position(|name| *name == first_capture)
                .map(|idx| idx as u32)
            else {
                return Vec::new();
            };
            let Some(second_index) = capture_names
                .iter()
                .position(|name| *name == second_capture)
                .map(|idx| idx as u32)
            else {
                return Vec::new();
            };

            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(query, self.tree.root_node(), self.source.as_slice());
            let mut pairs = Vec::new();
            while let Some(m) = matches.next() {
                let mut first = None;
                let mut second = None;
                for cap in m.captures {
                    if cap.index == first_index {
                        first = Some(cap.node);
                    } else if cap.index == second_index {
                        second = Some(cap.node);
                    }
                }
                if let (Some(first), Some(second)) = (first, second) {
                    pairs.push((first, second));
                }
            }
            pairs
        })
        .unwrap_or_default()
    }

    pub(crate) fn has_function_named(&self, function_name: &str, class_name: Option<&str>) -> bool {
        self.with_query(self.language_info.function_query, |query| {
            let capture_names = query.capture_names();
            let Some(function_index) = capture_names
                .iter()
                .position(|name| *name == "function")
                .map(|idx| idx as u32)
            else {
                return false;
            };
            let Some(name_index) = capture_names
                .iter()
                .position(|name| *name == "name")
                .map(|idx| idx as u32)
            else {
                return false;
            };

            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(query, self.tree.root_node(), self.source.as_slice());
            while let Some(m) = matches.next() {
                let mut function_node = None;
                let mut name_node = None;
                for cap in m.captures {
                    if cap.index == function_index {
                        function_node = Some(cap.node);
                    } else if cap.index == name_index {
                        name_node = Some(cap.node);
                    }
                }
                let (Some(function_node), Some(name_node)) = (function_node, name_node) else {
                    continue;
                };
                if !self.node_eq_str(name_node, function_name) {
                    continue;
                }
                if class_name.is_none()
                    || self.find_enclosing_class_name(function_node).as_deref() == class_name
                {
                    return true;
                }
            }
            false
        })
        .unwrap_or(false)
    }

    pub(crate) fn query_call_matches(&self, query_str: &str) -> Vec<CallQueryMatch<'_>> {
        self.with_query(query_str, |query| {
            let capture_names = query.capture_names();
            let call_index = capture_names
                .iter()
                .position(|name| *name == "call")
                .map(|idx| idx as u32);
            let callee_index = capture_names
                .iter()
                .position(|name| *name == "callee")
                .map(|idx| idx as u32);
            let method_index = capture_names
                .iter()
                .position(|name| *name == "method")
                .map(|idx| idx as u32);
            let object_index = capture_names
                .iter()
                .position(|name| *name == "object")
                .map(|idx| idx as u32);
            let Some(call_index) = call_index else {
                return Vec::new();
            };

            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(query, self.tree.root_node(), self.source.as_slice());
            let mut out = Vec::new();
            while let Some(m) = matches.next() {
                let mut call = None;
                let mut callee = None;
                let mut method = None;
                let mut object = None;
                for cap in m.captures {
                    if cap.index == call_index {
                        call = Some(cap.node);
                    } else if callee_index == Some(cap.index) {
                        callee = Some(cap.node);
                    } else if method_index == Some(cap.index) {
                        method = Some(cap.node);
                    } else if object_index == Some(cap.index) {
                        object = Some(cap.node);
                    }
                }
                if let Some(call) = call {
                    out.push(CallQueryMatch {
                        call,
                        callee,
                        method,
                        object,
                    });
                }
            }
            out
        })
        .unwrap_or_default()
    }

    pub(crate) fn is_function_like(node_kind: &str) -> bool {
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

    pub(crate) fn find_enclosing_function_name(&self, node: Node) -> Option<String> {
        self.find_enclosing_context(node).0
    }

    pub(crate) fn find_enclosing_function_node<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        self.find_enclosing_context(node).2
    }

    pub(crate) fn infer_anonymous_function_name(&self, func_node: Node) -> Option<String> {
        let parent = func_node.parent()?;
        match parent.kind() {
            "variable_declarator" => {
                let name_node = parent.child_by_field_name("name")?;
                if name_node.kind() == "identifier" {
                    return Some(self.node_text(name_node));
                }
            }
            "assignment_expression" | "assignment" => {
                let left_node = parent.child_by_field_name("left")?;
                if left_node.kind() == "identifier" {
                    return Some(self.node_text(left_node));
                }
            }
            "pair" | "property" => {
                let key_node = parent.child_by_field_name("key")?;
                if matches!(
                    key_node.kind(),
                    "identifier" | "property_identifier" | "string"
                ) {
                    let text = self.node_text_lossy(key_node);
                    return Some(text.trim_matches(|c| c == '"' || c == '\'').to_string());
                }
            }
            "export_statement" => {
                for i in 0..parent.child_count() {
                    let child = parent.child((i) as u32).unwrap();
                    if self.node_eq_str(child, "default") {
                        return Some("<default_export>".to_string());
                    }
                }
            }
            _ => {}
        }
        None
    }

    pub(crate) fn find_enclosing_class_name(&self, node: Node) -> Option<String> {
        self.find_enclosing_context(node).1
    }

    pub(crate) fn find_enclosing_context_names(
        &self,
        node: Node,
    ) -> (Option<String>, Option<String>) {
        let (function_name, class_name, _) = self.find_enclosing_context(node);
        (function_name, class_name)
    }

    pub(crate) fn find_enclosing_context<'a>(
        &self,
        node: Node<'a>,
    ) -> (Option<String>, Option<String>, Option<Node<'a>>) {
        if let Some(class_name) = self.find_language_specific_enclosing_class_name(node) {
            let function_name = self.get_cached_function_name(node);
            return (function_name, Some(class_name), Some(node));
        }

        let mut current = node.parent();
        let mut function_name: Option<String> = None;
        let mut class_name: Option<String> = None;
        let mut function_node: Option<Node<'a>> = None;
        while let Some(cur) = current {
            if function_name.is_none() && Self::is_function_like(cur.kind()) {
                function_name = self.get_cached_function_name(cur);
                function_node = Some(cur);
            }

            if class_name.is_none() {
                class_name = self.find_language_specific_enclosing_class_name(cur);
            }

            if class_name.is_none()
                && matches!(
                    cur.kind(),
                    "class_definition"
                        | "class_declaration"
                        | "class_body"
                        | "interface_declaration"
                        | "enum_declaration"
                        | "record_declaration"
                        | "annotation_type_declaration"
                )
            {
                class_name = self.get_cached_class_name(cur);
                // Some nodes like Java `class_body` don't carry the class name.
                // Keep walking upward to find the owning class declaration.
                current = cur.parent();
                continue;
            }
            current = cur.parent();
        }
        (function_name, class_name, function_node)
    }

    fn extract_function_name_from_node(&self, node: Node) -> Option<String> {
        let anonymous_types = ["arrow_function", "func_literal"];
        if let Some(name_node) = node.child_by_field_name("name") {
            return Some(self.node_text(name_node));
        }
        if anonymous_types.contains(&node.kind()) || node.kind() == "function_expression" {
            return self.infer_anonymous_function_name(node);
        }
        for i in 0..node.child_count() {
            let child = node.child((i) as u32).unwrap();
            if matches!(
                child.kind(),
                "identifier" | "property_identifier" | "field_identifier"
            ) {
                return Some(self.node_text(child));
            }
        }
        None
    }

    fn get_cached_function_name(&self, node: Node) -> Option<String> {
        let node_id = node.id();
        if let Some(name) = self.function_names_by_node.borrow().get(&node_id) {
            return name.clone();
        }
        let name = self.extract_function_name_from_node(node);
        self.function_names_by_node
            .borrow_mut()
            .insert(node_id, name.clone());
        name
    }

    fn extract_class_name_from_node(&self, node: Node) -> Option<String> {
        if let Some(name_node) = node.child_by_field_name("name") {
            let class_name = self.node_text(name_node);
            if !class_name.is_empty() {
                return Some(class_name);
            }
        }
        for i in 0..node.child_count() {
            let child = node.child((i) as u32).unwrap();
            if matches!(child.kind(), "identifier" | "type_identifier" | "name") {
                let class_name = self.node_text(child);
                if !class_name.is_empty() {
                    return Some(class_name);
                }
            }
        }
        None
    }

    fn get_cached_class_name(&self, node: Node) -> Option<String> {
        let node_id = node.id();
        if let Some(name) = self.class_names_by_node.borrow().get(&node_id) {
            return name.clone();
        }
        let name = self.extract_class_name_from_node(node);
        self.class_names_by_node
            .borrow_mut()
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn extract_attribute_parts(&self, node: Node) -> (String, Option<String>) {
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

    pub(crate) fn extract_attribute_callee_name(&self, node: Node) -> Option<String> {
        let callee_node = match node.kind() {
            "attribute" => node.child_by_field_name("attribute"),
            "member_expression" => node.child_by_field_name("property"),
            "selector_expression" => node.child_by_field_name("field"),
            _ => {
                let mut last_identifier = None;
                for i in 0..node.named_child_count() {
                    let Some(child) = node.named_child(i as u32) else {
                        continue;
                    };
                    if matches!(
                        child.kind(),
                        "identifier"
                            | "property_identifier"
                            | "private_property_identifier"
                            | "field_identifier"
                    ) {
                        last_identifier = Some(child);
                    }
                }
                last_identifier
            }
        }?;

        let callee = self.node_text(callee_node);
        if callee.is_empty() {
            None
        } else {
            Some(callee)
        }
    }

    pub(crate) fn extract_function_params(&self, function_node: Node) -> Vec<FunctionParamInfo> {
        let target = self.unwrap_callable_node(function_node);
        match self.language.as_str() {
            "python" => self.extract_python_function_params(target),
            "javascript" | "typescript" | "tsx" => self.extract_js_like_function_params(target),
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

    pub(super) fn find_first_identifier_text(&self, node: Node) -> String {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if matches!(current.kind(), "identifier" | "keyword_identifier") {
                let text = self.node_text(current);
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

    pub(super) fn extract_declared_field_infos(
        &self,
        class_node: Node,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        self.collect_field_infos_from_declarations(class_node, class_name, false)
    }

    pub(super) fn extract_declared_field_infos_with_embedded_type_names(
        &self,
        class_node: Node,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        self.collect_field_infos_from_declarations(class_node, class_name, true)
    }

    fn collect_field_infos_from_declarations(
        &self,
        class_node: Node,
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
                    .map(|n| self.node_text(n))
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
                let mut field_type = node.child_by_field_name("type").map(|n| self.node_text(n));

                for i in 0..node.child_count() {
                    let child = node.child(i as u32).unwrap();
                    if matches!(
                        child.kind(),
                        "identifier" | "property_identifier" | "field_identifier"
                    ) {
                        names.push(self.node_text(child));
                    } else if child.kind() == "variable_declarator" {
                        for j in 0..child.child_count() {
                            let sub = child.child(j as u32).unwrap();
                            if sub.kind() == "identifier" {
                                names.push(self.node_text(sub));
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
                        field_type = Some(self.node_text(child));
                    }
                }

                if include_embedded_type_names && names.is_empty() {
                    if let Some(ref ft) = field_type {
                        let type_str = ft.trim_start_matches('*');
                        let name = if type_str.contains('.') {
                            type_str.rsplit('.').next().unwrap_or(type_str)
                        } else {
                            type_str
                        };
                        names.push(name.to_string());
                    }
                }

                for name in names {
                    if !name.is_empty() && seen.insert(name.clone()) {
                        fields.push(FieldInfo {
                            name,
                            location: self.node_location(node),
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

    pub(crate) fn extract_method_names_from_class(&self, class_node: Node) -> Vec<String> {
        let mut methods = Vec::new();
        let mut stack = vec![class_node];
        while let Some(node) = stack.pop() {
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
                    let child = node.child((i) as u32).unwrap();
                    if matches!(
                        child.kind(),
                        "identifier" | "property_identifier" | "field_identifier" | "name"
                    ) {
                        methods.push(self.node_text(child));
                        break;
                    }
                }
                continue;
            }
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child((i) as u32) {
                    stack.push(child);
                }
            }
        }
        methods
    }

    pub(crate) fn extract_field_names_from_class(&self, class_node: Node) -> Vec<String> {
        let class_name_node = class_node.child_by_field_name("name");
        let class_name = class_name_node
            .map(|n| self.node_text(n))
            .unwrap_or_default();
        self.extract_field_infos_from_class(class_node, &class_name)
            .into_iter()
            .map(|f| f.name)
            .collect()
    }

    pub(crate) fn extract_field_infos_from_class(
        &self,
        class_node: Node,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        match self.language.as_str() {
            "python" => self.extract_python_field_infos(class_node, class_name),
            "javascript" | "typescript" | "tsx" => {
                self.extract_js_like_field_infos(class_node, class_name)
            }
            "java" => self.extract_java_field_infos(class_node, class_name),
            "go" => self.extract_go_field_infos(class_node, class_name),
            _ => Vec::new(),
        }
    }

    pub(crate) fn extract_super_class_names(&self, class_node: Node) -> Vec<String> {
        match self.language.as_str() {
            "python" => self.extract_python_super_class_names(class_node),
            "javascript" | "typescript" | "tsx" => {
                self.extract_js_like_super_class_names(class_node)
            }
            "java" => self.extract_java_super_class_names(class_node),
            "go" => self.extract_go_super_class_names(class_node),
            _ => Vec::new(),
        }
    }

    pub(crate) fn extract_annotations(&self) -> Vec<AnnotationInfo> {
        match self.language.as_str() {
            "java" => self.extract_java_annotations(),
            "python" => self.extract_python_decorators(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn find_field_infos_by_class_name(&self, class_name: &str) -> Vec<FieldInfo> {
        let matches = self.query_capture_pairs(self.language_info.class_query, "class", "name");
        let mut candidates: Vec<Node> = Vec::new();

        for (class_node, name_node) in matches {
            if !self.node_eq_str(name_node, class_name) {
                continue;
            }
            candidates.push(class_node);
        }
        if candidates.is_empty() {
            return Vec::new();
        }

        candidates.sort_by_key(|n| {
            let size = n.end_byte() - n.start_byte();
            (size, n.start_byte())
        });
        let target = candidates[0];
        self.extract_field_infos_from_class(target, class_name)
    }
}
