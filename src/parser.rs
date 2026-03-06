use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor, Tree};

use crate::languages::{
    detect_language, get_compiled_query, get_language, get_language_info, LanguageInfo,
};
use crate::nodes::*;

pub(crate) struct BaseParser {
    pub(crate) file_path: String,
    pub(crate) language: String,
    pub(crate) source: Vec<u8>,
    pub(crate) tree: Tree,
    pub(crate) lang_info: &'static LanguageInfo,
    function_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    class_names_by_node: RefCell<HashMap<usize, Option<String>>>,
}

pub(crate) struct CallQueryMatch<'a> {
    pub(crate) call: Node<'a>,
    pub(crate) callee: Option<Node<'a>>,
    pub(crate) method: Option<Node<'a>>,
    pub(crate) object: Option<Node<'a>>,
}

pub(crate) struct JsAliasEvent {
    pub(crate) start_byte: usize,
    pub(crate) name: String,
    pub(crate) targets: Vec<String>,
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
        let ts_lang = get_language(language)
            .ok_or_else(|| anyhow::anyhow!("Unsupported language: {}", language))?;
        let lang_info = get_language_info(language)
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
            lang_info,
            function_names_by_node: RefCell::new(HashMap::new()),
            class_names_by_node: RefCell::new(HashMap::new()),
        })
    }

    pub(crate) fn node_text(&self, node: Node) -> String {
        self.node_text_cow(node).into_owned()
    }

    pub(crate) fn node_bytes<'a>(&'a self, node: Node) -> &'a [u8] {
        &self.source[node.start_byte()..node.end_byte()]
    }

    pub(crate) fn node_text_cow<'a>(&'a self, node: Node) -> Cow<'a, str> {
        String::from_utf8_lossy(self.node_bytes(node))
    }

    pub(crate) fn node_eq_str(&self, node: Node, text: &str) -> bool {
        self.node_bytes(node) == text.as_bytes()
    }

    pub(crate) fn node_trimmed_eq_str(&self, node: Node, text: &str) -> bool {
        self.node_text_cow(node).trim() == text
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
        self.node_text_cow(node)
    }

    pub(crate) fn node_text_utf8(&self, node: Node) -> String {
        self.node_text(node)
    }

    pub(crate) fn node_location(&self, node: Node) -> Location {
        Location {
            file: self.file_path.clone(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        }
    }

    fn with_query<T>(&self, query_str: &str, f: impl FnOnce(&Query) -> T) -> Option<T> {
        if let Some(query) = get_compiled_query(&self.language, query_str) {
            return Some(f(query));
        }
        let ts_lang = get_language(&self.language)?;
        let query = match Query::new(&ts_lang, query_str) {
            Ok(q) => q,
            Err(_) => return None,
        };
        Some(f(&query))
    }

    pub(crate) fn run_query_capture(&self, query_str: &str, capture_name: &str) -> Vec<Node<'_>> {
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

    pub(crate) fn run_query_pairs(
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
        self.with_query(self.lang_info.function_query, |query| {
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
                    || self.find_enclosing_class(function_node).as_deref() == class_name
                {
                    return true;
                }
            }
            false
        })
        .unwrap_or(false)
    }

    pub(crate) fn run_call_query_matches(&self, query_str: &str) -> Vec<CallQueryMatch<'_>> {
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

    pub(crate) fn find_enclosing_function(&self, node: Node) -> Option<String> {
        self.find_enclosing_context_with_function_node(node).0
    }

    pub(crate) fn find_enclosing_function_node<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        self.find_enclosing_context_with_function_node(node).2
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
                    let text = self.node_text_cow(key_node);
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

    pub(crate) fn find_enclosing_class(&self, node: Node) -> Option<String> {
        self.find_enclosing_context_with_function_node(node).1
    }

    pub(crate) fn find_enclosing_context(&self, node: Node) -> (Option<String>, Option<String>) {
        let (function_name, class_name, _) = self.find_enclosing_context_with_function_node(node);
        (function_name, class_name)
    }

    pub(crate) fn find_enclosing_context_with_function_node<'a>(
        &self,
        node: Node<'a>,
    ) -> (Option<String>, Option<String>, Option<Node<'a>>) {
        if self.language == "go" && node.kind() == "method_declaration" {
            if let Some(rc) = self.extract_go_receiver_type(node) {
                let function_name = self.cached_function_name_from_node(node);
                return (function_name, Some(rc), Some(node));
            }
        }

        let mut current = node.parent();
        let mut function_name: Option<String> = None;
        let mut class_name: Option<String> = None;
        let mut function_node: Option<Node<'a>> = None;
        while let Some(cur) = current {
            if function_name.is_none() && Self::is_function_like(cur.kind()) {
                function_name = self.cached_function_name_from_node(cur);
                function_node = Some(cur);
            }

            if self.language == "go" && class_name.is_none() && cur.kind() == "method_declaration" {
                class_name = self.extract_go_receiver_type(cur);
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
                class_name = self.cached_class_name_from_node(cur);
                // Some nodes like Java `class_body` don't carry the class name.
                // Keep walking upward to find the owning class declaration.
                current = cur.parent();
                continue;
            }
            current = cur.parent();
        }
        (function_name, class_name, function_node)
    }

    fn function_name_from_node(&self, node: Node) -> Option<String> {
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

    fn cached_function_name_from_node(&self, node: Node) -> Option<String> {
        let node_id = node.id();
        if let Some(name) = self.function_names_by_node.borrow().get(&node_id) {
            return name.clone();
        }
        let name = self.function_name_from_node(node);
        self.function_names_by_node
            .borrow_mut()
            .insert(node_id, name.clone());
        name
    }

    fn class_name_from_node(&self, node: Node) -> Option<String> {
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

    fn cached_class_name_from_node(&self, node: Node) -> Option<String> {
        let node_id = node.id();
        if let Some(name) = self.class_names_by_node.borrow().get(&node_id) {
            return name.clone();
        }
        let name = self.class_name_from_node(node);
        self.class_names_by_node
            .borrow_mut()
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn extract_go_receiver_type(&self, method_node: Node) -> Option<String> {
        let receiver_list = method_node.child_by_field_name("receiver").or_else(|| {
            for i in 0..method_node.child_count() {
                let child = method_node.child((i) as u32).unwrap();
                if child.kind() == "parameter_list" {
                    return Some(child);
                }
            }
            None
        })?;

        for i in 0..receiver_list.child_count() {
            let param = receiver_list.child((i) as u32).unwrap();
            if param.kind() == "parameter_declaration" {
                if let Some(type_node) = param.child_by_field_name("type") {
                    return self.unwrap_go_type(type_node);
                }
                for j in 0..param.child_count() {
                    let p = param.child((j) as u32).unwrap();
                    if p.kind() == "pointer_type" {
                        for k in 0..p.child_count() {
                            let pt = p.child((k) as u32).unwrap();
                            if pt.kind() == "type_identifier" {
                                return Some(self.node_text(pt));
                            }
                        }
                    }
                    if p.kind() == "type_identifier" {
                        return Some(self.node_text(p));
                    }
                }
            }
        }
        None
    }

    pub(crate) fn unwrap_go_type(&self, type_node: Node) -> Option<String> {
        match type_node.kind() {
            "type_identifier" => Some(self.node_text(type_node)),
            "pointer_type" => {
                for i in 0..type_node.child_count() {
                    let child = type_node.child((i) as u32).unwrap();
                    if child.kind() != "*" {
                        return self.unwrap_go_type(child);
                    }
                }
                None
            }
            "generic_type" => {
                if let Some(base) = type_node.child_by_field_name("type") {
                    return self.unwrap_go_type(base);
                }
                for i in 0..type_node.child_count() {
                    let child = type_node.child((i) as u32).unwrap();
                    if child.kind() == "type_identifier" {
                        return Some(self.node_text(child));
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn parse_attribute_node(&self, node: Node) -> (String, Option<String>) {
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

    pub(crate) fn attribute_callee_name(&self, node: Node) -> Option<String> {
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

    pub(crate) fn resolve_js_identifier_call_targets(
        &self,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        let Some(func_node) = self.find_enclosing_function_node(call_node) else {
            return Vec::new();
        };

        let events = self.build_js_alias_events(func_node);
        self.resolve_js_identifier_call_targets_from_events(
            &events,
            call_node.start_byte(),
            identifier_name,
        )
    }

    pub(crate) fn build_js_alias_events(&self, func_node: Node<'_>) -> Vec<JsAliasEvent> {
        let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
        let mut events = Vec::new();

        fn add_alias(
            aliases: &mut HashMap<String, Vec<String>>,
            name: Option<String>,
            targets: impl IntoIterator<Item = String>,
        ) -> Option<(String, Vec<String>)> {
            let name = name?;
            if name.is_empty() {
                return None;
            }
            let entry = aliases.entry(name.clone()).or_default();
            for target in targets {
                if !target.is_empty() {
                    entry.push(target);
                }
            }
            entry.sort_unstable();
            entry.dedup();
            Some((name, entry.clone()))
        }

        let extract_loop_var = |node: Node<'_>| -> Option<String> {
            if node.kind() == "identifier" {
                return Some(self.node_text(node));
            }
            for i in 0..node.named_child_count() {
                let Some(child) = node.named_child(i as u32) else {
                    continue;
                };
                if child.kind() == "identifier" {
                    return Some(self.node_text(child));
                }
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if name_node.kind() == "identifier" {
                            return Some(self.node_text(name_node));
                        }
                    }
                }
            }
            None
        };

        let mut stack = vec![func_node];
        while let Some(node) = stack.pop() {
            if node.kind() == "variable_declarator" {
                let name_node = node.child_by_field_name("name");
                let value_node = node.child_by_field_name("value");
                if let (Some(name_node), Some(value_node)) = (name_node, value_node) {
                    if name_node.kind() == "identifier" {
                        let name = self.node_text(name_node);
                        if value_node.kind() == "identifier" {
                            if let Some((name, targets)) =
                                add_alias(&mut aliases, Some(name), [self.node_text(value_node)])
                            {
                                events.push(JsAliasEvent {
                                    start_byte: node.start_byte(),
                                    name,
                                    targets,
                                });
                            }
                        } else if value_node.kind() == "array" {
                            let targets = (0..value_node.named_child_count())
                                .filter_map(|i| value_node.named_child(i as u32))
                                .filter(|c| c.kind() == "identifier")
                                .map(|c| self.node_text(c))
                                .collect::<Vec<_>>();
                            if let Some((name, targets)) =
                                add_alias(&mut aliases, Some(name), targets)
                            {
                                events.push(JsAliasEvent {
                                    start_byte: node.start_byte(),
                                    name,
                                    targets,
                                });
                            }
                        }
                    }
                }
            } else if node.kind() == "for_in_statement" {
                let left_node = node.child_by_field_name("left");
                let right_node = node.child_by_field_name("right");
                if let (Some(left_node), Some(right_node)) = (left_node, right_node) {
                    if right_node.kind() == "identifier" {
                        let loop_var = extract_loop_var(left_node);
                        let iterable = self.node_text(right_node);
                        if let Some(targets) = aliases.get(&iterable).cloned() {
                            if let Some((name, targets)) =
                                add_alias(&mut aliases, loop_var, targets)
                            {
                                events.push(JsAliasEvent {
                                    start_byte: node.start_byte(),
                                    name,
                                    targets,
                                });
                            }
                        } else if let Some((name, targets)) =
                            add_alias(&mut aliases, loop_var, [iterable])
                        {
                            events.push(JsAliasEvent {
                                start_byte: node.start_byte(),
                                name,
                                targets,
                            });
                        }
                    }
                }
            }

            for i in (0..node.named_child_count()).rev() {
                if let Some(child) = node.named_child(i as u32) {
                    stack.push(child);
                }
            }
        }

        events
    }

    pub(crate) fn resolve_js_identifier_call_targets_from_events(
        &self,
        events: &[JsAliasEvent],
        call_start: usize,
        identifier_name: &str,
    ) -> Vec<String> {
        let mut aliases: HashMap<&str, &[String]> = HashMap::new();
        for event in events {
            if event.start_byte >= call_start {
                break;
            }
            aliases.insert(event.name.as_str(), event.targets.as_slice());
        }

        let mut visited: HashSet<&str> = HashSet::new();
        let mut resolved = Vec::new();
        let mut resolved_seen: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(identifier_name);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(targets) = aliases.get(current) {
                for target in *targets {
                    queue.push_back(target.as_str());
                }
            } else if resolved_seen.insert(current) {
                resolved.push(current);
            }
        }

        resolved.sort_unstable();
        resolved.into_iter().map(str::to_string).collect()
    }

    pub(crate) fn build_python_property_index(
        &self,
    ) -> (PythonPropertyDefinitions, PythonPropertyCallers) {
        if self.language != "python" {
            return (HashSet::new(), HashMap::new());
        }

        let mut properties = HashSet::new();
        let mut callers_by_property: HashMap<String, Vec<(String, usize)>> = HashMap::new();
        let mut seen_callers: HashSet<(String, String, usize)> = HashSet::new();
        let mut stack = vec![self.tree.root_node()];

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
                            && self.node_trimmed_eq_str(child, "@property")
                        {
                            is_property = true;
                            break;
                        }
                    }

                    if is_property {
                        properties.insert((
                            self.node_text(name_node),
                            self.find_enclosing_class(definition_node),
                        ));
                    }
                }
                "attribute" => {
                    let Some(property_name) = self.attribute_callee_name(node) else {
                        continue;
                    };
                    let caller = self
                        .find_enclosing_function(node)
                        .unwrap_or_else(|| "<module>".to_string());
                    let line = node.start_position().row + 1;
                    let seen_key = (property_name.clone(), caller.clone(), line);
                    if seen_callers.insert(seen_key) {
                        callers_by_property
                            .entry(property_name)
                            .or_default()
                            .push((caller, line));
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

    pub(crate) fn is_python_property(&self, function_name: &str, class_name: Option<&str>) -> bool {
        let (properties, _) = self.build_python_property_index();
        properties.contains(&(function_name.to_string(), class_name.map(str::to_string)))
    }

    pub(crate) fn find_python_property_callers(&self, property_name: &str) -> Vec<(String, usize)> {
        let (_, callers_by_property) = self.build_python_property_index();
        callers_by_property
            .get(property_name)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn extract_methods_from_class(&self, class_node: Node) -> Vec<String> {
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

    pub(crate) fn extract_fields_from_class(&self, class_node: Node) -> Vec<String> {
        let class_name_node = class_node.child_by_field_name("name");
        let class_name = class_name_node
            .map(|n| self.node_text(n))
            .unwrap_or_default();
        self.extract_field_infos(class_node, &class_name)
            .into_iter()
            .map(|f| f.name)
            .collect()
    }

    pub(crate) fn extract_field_infos(&self, class_node: Node, class_name: &str) -> Vec<FieldInfo> {
        if matches!(self.language.as_str(), "javascript" | "typescript" | "tsx") {
            return self.extract_field_infos_js_like(class_node, class_name);
        }

        let fields: Vec<FieldInfo> = Vec::new();
        let seen: HashSet<String> = HashSet::new();
        struct WalkCtx<'a> {
            analyzer: &'a BaseParser,
            class_node_id: usize,
            class_name: String,
            fields: Vec<FieldInfo>,
            seen: HashSet<String>,
        }

        fn walk(ctx: &mut WalkCtx, node: Node, inside_method: bool) {
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
                    .map(|n| ctx.analyzer.node_text(n))
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
                if ctx.analyzer.language != "python" {
                    return;
                }
                for i in 0..node.child_count() {
                    if let Some(child) = node.child((i) as u32) {
                        walk(ctx, child, true);
                    }
                }
                return;
            }

            if matches!(node.kind(), "field_definition" | "field_declaration") {
                let mut names = Vec::new();
                let mut field_type: Option<String> = None;

                if let Some(type_node) = node.child_by_field_name("type") {
                    field_type = Some(ctx.analyzer.node_text(type_node));
                }

                for i in 0..node.child_count() {
                    let child = node.child((i) as u32).unwrap();
                    if matches!(
                        child.kind(),
                        "identifier" | "property_identifier" | "field_identifier"
                    ) {
                        names.push(ctx.analyzer.node_text(child));
                    } else if child.kind() == "variable_declarator" {
                        for j in 0..child.child_count() {
                            let sub = child.child((j) as u32).unwrap();
                            if sub.kind() == "identifier" {
                                names.push(ctx.analyzer.node_text(sub));
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
                        field_type = Some(ctx.analyzer.node_text(child));
                    }
                }

                // Go embedded fields
                if ctx.analyzer.language == "go" && names.is_empty() {
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
                    if !name.is_empty() && ctx.seen.insert(name.clone()) {
                        ctx.fields.push(FieldInfo {
                            name,
                            location: ctx.analyzer.node_location(node),
                            field_type: field_type.clone(),
                            class_name: Some(ctx.class_name.clone()),
                        });
                    }
                }
                return;
            }

            // Python: expression_statement with assignment
            if ctx.analyzer.language == "python" && node.kind() == "expression_statement" {
                for i in 0..node.child_count() {
                    let child = node.child((i) as u32).unwrap();
                    if child.kind() == "assignment" {
                        let left_node = match child.child_by_field_name("left") {
                            Some(n) => n,
                            None => continue,
                        };
                        let mut field_type: Option<String> = None;
                        if let Some(type_node) = child.child_by_field_name("type") {
                            field_type = Some(ctx.analyzer.node_text(type_node));
                        }
                        let mut name = String::new();
                        if inside_method {
                            if left_node.kind() == "attribute" {
                                let obj_node = left_node.child_by_field_name("object");
                                let attr_node = left_node.child_by_field_name("attribute");
                                if let (Some(obj), Some(attr)) = (obj_node, attr_node) {
                                    if ctx.analyzer.node_eq_str(obj, "self") {
                                        name = ctx.analyzer.node_text(attr);
                                    }
                                }
                            }
                        } else if left_node.kind() == "identifier" {
                            name = ctx.analyzer.node_text(left_node);
                        }
                        if !name.is_empty() && ctx.seen.insert(name.clone()) {
                            ctx.fields.push(FieldInfo {
                                name,
                                location: ctx.analyzer.node_location(child),
                                field_type,
                                class_name: Some(ctx.class_name.clone()),
                            });
                        }
                    }
                }
                return;
            }

            for i in 0..node.child_count() {
                if let Some(child) = node.child((i) as u32) {
                    walk(ctx, child, inside_method);
                }
            }
        }

        let mut ctx = WalkCtx {
            analyzer: self,
            class_node_id: class_node.id(),
            class_name: class_name.to_string(),
            fields,
            seen,
        };
        walk(&mut ctx, class_node, false);
        ctx.fields
    }

    pub(crate) fn extract_field_infos_js_like(
        &self,
        class_node: Node,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        let body = class_node.child_by_field_name("body").or_else(|| {
            for i in 0..class_node.child_count() {
                let child = class_node.child((i) as u32).unwrap();
                if child.kind() == "class_body" {
                    return Some(child);
                }
            }
            None
        });
        let body = match body {
            Some(b) => b,
            None => return Vec::new(),
        };

        let mut fields: Vec<FieldInfo> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for i in 0..body.child_count() {
            let member = body.child((i) as u32).unwrap();
            if !member.is_named() {
                continue;
            }

            if member.kind().ends_with("field_definition")
                || matches!(member.kind(), "property_definition" | "field_definition")
            {
                let name_node = member
                    .child_by_field_name("name")
                    .or_else(|| member.child_by_field_name("property"))
                    .or_else(|| member.child_by_field_name("pattern"));
                let name_node = match name_node {
                    Some(n) => n,
                    None => continue,
                };
                if !matches!(
                    name_node.kind(),
                    "identifier"
                        | "property_identifier"
                        | "private_property_identifier"
                        | "field_identifier"
                ) {
                    continue;
                }
                let name = self.node_text(name_node);
                let field_type = member
                    .child_by_field_name("type")
                    .map(|t| self.clean_type_text(&self.node_text(t)));
                if !name.is_empty() && seen.insert(name.clone()) {
                    fields.push(FieldInfo {
                        name,
                        location: self.node_location(member),
                        field_type,
                        class_name: Some(class_name.to_string()),
                    });
                }
            }

            if member.kind() == "method_definition" {
                let name_node = match member.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };
                if !self.node_eq_str(name_node, "constructor") {
                    continue;
                }
                let params = match member.child_by_field_name("parameters") {
                    Some(p) => p,
                    None => continue,
                };
                for j in 0..params.child_count() {
                    let param = params.child((j) as u32).unwrap();
                    if !param.is_named() {
                        continue;
                    }
                    let has_modifier = (0..param.child_count()).any(|k| {
                        let c = param.child((k) as u32).unwrap();
                        matches!(c.kind(), "accessibility_modifier" | "readonly")
                    });
                    if !has_modifier {
                        continue;
                    }
                    let mut pattern = param
                        .child_by_field_name("pattern")
                        .or_else(|| param.child_by_field_name("name"));
                    if let Some(p) = pattern {
                        if p.kind() == "assignment_pattern" {
                            pattern = p.child_by_field_name("left").or(Some(p));
                        }
                    }
                    let pattern = match pattern {
                        Some(p) => p,
                        None => continue,
                    };
                    if !matches!(pattern.kind(), "identifier" | "property_identifier") {
                        continue;
                    }
                    let param_name = self.node_text(pattern);
                    let param_type = param
                        .child_by_field_name("type")
                        .map(|t| self.clean_type_text(&self.node_text(t)));
                    if !param_name.is_empty() && seen.insert(param_name.clone()) {
                        fields.push(FieldInfo {
                            name: param_name,
                            location: self.node_location(param),
                            field_type: param_type,
                            class_name: Some(class_name.to_string()),
                        });
                    }
                }

                // Scan constructor body for this.prop assignments
                if let Some(body_node) = member.child_by_field_name("body") {
                    self.extract_fields_from_constructor_body(
                        body_node,
                        class_name,
                        &mut seen,
                        &mut fields,
                    );
                }
            }
        }
        fields
    }

    pub(crate) fn extract_fields_from_constructor_body(
        &self,
        body_node: Node,
        class_name: &str,
        seen: &mut HashSet<String>,
        fields: &mut Vec<FieldInfo>,
    ) {
        let mut stack = vec![body_node];
        while let Some(node) = stack.pop() {
            if node.kind() == "assignment_expression" {
                if let Some(left) = node.child_by_field_name("left") {
                    if left.kind() == "member_expression" {
                        let obj = left.child_by_field_name("object");
                        let prop = left.child_by_field_name("property");
                        if let (Some(obj), Some(prop)) = (obj, prop) {
                            if self.node_eq_str(obj, "this") {
                                let name = self.node_text(prop);
                                if !name.is_empty() && seen.insert(name.clone()) {
                                    fields.push(FieldInfo {
                                        name,
                                        location: self.node_location(node),
                                        field_type: None,
                                        class_name: Some(class_name.to_string()),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            if matches!(
                node.kind(),
                "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "method_definition"
                    | "class_declaration"
                    | "class_expression"
            ) && node.id() != body_node.id()
            {
                continue;
            }
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child((i) as u32) {
                    stack.push(child);
                }
            }
        }
    }

    pub(crate) fn clean_type_text(&self, text: &str) -> String {
        let stripped = text.trim();
        if let Some(rest) = stripped.strip_prefix(':') {
            rest.trim().to_string()
        } else {
            stripped.to_string()
        }
    }

    pub(crate) fn extract_super_classes(&self, class_node: Node) -> Vec<String> {
        let mut super_classes = Vec::new();

        match self.language.as_str() {
            "python" => {
                for i in 0..class_node.child_count() {
                    let child = class_node.child((i) as u32).unwrap();
                    if child.kind() == "argument_list" {
                        for j in 0..child.child_count() {
                            let arg = child.child((j) as u32).unwrap();
                            if matches!(arg.kind(), "identifier" | "attribute") {
                                super_classes.push(self.node_text(arg));
                            }
                        }
                    }
                }
            }
            "javascript" | "typescript" | "tsx" => {
                let mut seen: HashSet<String> = HashSet::new();
                self.walk_heritage(class_node, &mut super_classes, &mut seen);
            }
            "java" => {
                for i in 0..class_node.child_count() {
                    let child = class_node.child((i) as u32).unwrap();
                    match child.kind() {
                        "superclass" => {
                            for j in 0..child.child_count() {
                                let sub = child.child((j) as u32).unwrap();
                                if sub.kind() == "type_identifier" {
                                    super_classes.push(self.node_text(sub));
                                } else if sub.kind() == "generic_type" {
                                    for k in 0..sub.child_count() {
                                        let g = sub.child((k) as u32).unwrap();
                                        if g.kind() == "type_identifier" {
                                            super_classes.push(self.node_text(g));
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        "super_interfaces" => {
                            for j in 0..child.child_count() {
                                let sub = child.child((j) as u32).unwrap();
                                if sub.kind() == "type_list" {
                                    for k in 0..sub.child_count() {
                                        let t = sub.child((k) as u32).unwrap();
                                        if t.kind() == "type_identifier" {
                                            super_classes.push(self.node_text(t));
                                        } else if t.kind() == "generic_type" {
                                            for l in 0..t.child_count() {
                                                let g = t.child((l) as u32).unwrap();
                                                if g.kind() == "type_identifier" {
                                                    super_classes.push(self.node_text(g));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "go" => {
                self.extract_go_super_classes(class_node, &mut super_classes);
            }
            _ => {}
        }
        super_classes
    }

    pub(crate) fn walk_heritage(
        &self,
        node: Node,
        super_classes: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        for i in 0..node.child_count() {
            let child = node.child((i) as u32).unwrap();
            if child.kind() == "class_heritage" {
                for j in 0..child.child_count() {
                    let sub = child.child((j) as u32).unwrap();
                    if matches!(sub.kind(), "extends_clause" | "implements_clause") {
                        for k in 0..sub.child_count() {
                            let gc = sub.child((k) as u32).unwrap();
                            if gc.is_named() {
                                self.handle_heritage_expression(gc, super_classes, seen);
                            }
                        }
                    } else if sub.is_named() {
                        self.handle_heritage_expression(sub, super_classes, seen);
                    }
                }
            }
        }
    }

    pub(crate) fn handle_heritage_expression(
        &self,
        node: Node,
        super_classes: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        match node.kind() {
            "identifier" | "type_identifier" | "property_identifier" => {
                let name = self.node_text(node).trim().to_string();
                if !name.is_empty() && seen.insert(name.clone()) {
                    super_classes.push(name);
                }
            }
            "member_expression" => {
                let text = self.node_text(node).trim().to_string();
                if !text.is_empty() && seen.insert(text.clone()) {
                    super_classes.push(text);
                }
                for i in (0..node.child_count()).rev() {
                    let c = node.child((i) as u32).unwrap();
                    if matches!(c.kind(), "identifier" | "property_identifier") {
                        let name = self.node_text(c).trim().to_string();
                        if !name.is_empty() && seen.insert(name.clone()) {
                            super_classes.push(name);
                        }
                        break;
                    }
                }
            }
            "expression_with_type_arguments" => {
                if let Some(expr) = node.child_by_field_name("expression") {
                    self.handle_heritage_expression(expr, super_classes, seen);
                    return;
                }
                for i in 0..node.named_child_count() {
                    if let Some(c) = node.named_child((i) as u32) {
                        self.handle_heritage_expression(c, super_classes, seen);
                        return;
                    }
                }
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    self.handle_heritage_expression(func, super_classes, seen);
                }
            }
            _ => {
                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child((i) as u32) {
                        self.handle_heritage_expression(child, super_classes, seen);
                    }
                }
            }
        }
    }

    pub(crate) fn extract_go_super_classes(
        &self,
        class_node: Node,
        super_classes: &mut Vec<String>,
    ) {
        for i in 0..class_node.child_count() {
            let child = class_node.child((i) as u32).unwrap();
            if child.kind() == "type_spec" {
                for j in 0..child.child_count() {
                    let sub = child.child((j) as u32).unwrap();
                    if sub.kind() == "struct_type" {
                        for k in 0..sub.child_count() {
                            let field = sub.child((k) as u32).unwrap();
                            if field.kind() == "field_declaration_list" {
                                for l in 0..field.child_count() {
                                    let fd = field.child((l) as u32).unwrap();
                                    if fd.kind() == "field_declaration" {
                                        if let Some(embedded) =
                                            self.embedded_from_field_declaration(fd)
                                        {
                                            super_classes.push(embedded);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn embedded_from_field_declaration(&self, fd: Node) -> Option<String> {
        // If there's a field_identifier, it's a named field, not embedded
        for i in 0..fd.child_count() {
            let c = fd.child((i) as u32).unwrap();
            if c.kind() == "field_identifier" {
                return None;
            }
        }
        for i in 0..fd.child_count() {
            let c = fd.child((i) as u32).unwrap();
            if c.kind() == "*" {
                continue;
            }
            if matches!(
                c.kind(),
                "type_identifier"
                    | "qualified_type"
                    | "generic_type"
                    | "pointer_type"
                    | "parenthesized_type"
            ) {
                return self.parse_embedded_type_name(c);
            }
        }
        for i in 0..fd.named_child_count() {
            if let Some(c) = fd.named_child((i) as u32) {
                if let Some(name) = self.parse_embedded_type_name(c) {
                    return Some(name);
                }
            }
        }
        None
    }

    pub(crate) fn parse_embedded_type_name(&self, node: Node) -> Option<String> {
        match node.kind() {
            "type_identifier" => Some(self.node_text(node)),
            "qualified_type" => {
                for i in 0..node.child_count() {
                    let c = node.child((i) as u32).unwrap();
                    if c.kind() == "type_identifier" {
                        return Some(self.node_text(c));
                    }
                }
                None
            }
            "generic_type" => {
                for i in 0..node.child_count() {
                    let c = node.child((i) as u32).unwrap();
                    if matches!(
                        c.kind(),
                        "type_identifier" | "qualified_type" | "pointer_type"
                    ) {
                        return self.parse_embedded_type_name(c);
                    }
                }
                None
            }
            "pointer_type" => {
                for i in 0..node.child_count() {
                    let c = node.child((i) as u32).unwrap();
                    if matches!(
                        c.kind(),
                        "type_identifier"
                            | "qualified_type"
                            | "generic_type"
                            | "parenthesized_type"
                    ) {
                        return self.parse_embedded_type_name(c);
                    }
                }
                None
            }
            "parenthesized_type" => {
                for i in 0..node.named_child_count() {
                    if let Some(c) = node.named_child((i) as u32) {
                        if let Some(name) = self.parse_embedded_type_name(c) {
                            return Some(name);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn get_fields_from_class_node(&self, class_name: &str) -> Vec<FieldInfo> {
        let matches = self.run_query_pairs(self.lang_info.class_query, "class", "name");
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
        self.extract_field_infos(target, class_name)
    }
}
