use std::collections::{HashMap, HashSet, VecDeque};

use tree_sitter::Node;

use crate::cache::AnalyzerCache;
use crate::nodes::*;
use crate::parser::BaseParser;

pub struct CodeAnalyzer {
    pub(crate) parser: BaseParser,
    cache: AnalyzerCache,
}

#[allow(dead_code)]
impl CodeAnalyzer {
    pub fn new(file_path: &str) -> anyhow::Result<Self> {
        let parser = BaseParser::new(file_path)?;
        Ok(Self {
            parser,
            cache: AnalyzerCache::new(),
        })
    }

    fn build_functions(&self, include_body: bool) -> Vec<FunctionInfo> {
        let mut func_pairs =
            self.parser
                .run_query_pairs(self.parser.lang_info.function_query, "function", "name");
        func_pairs.sort_by_key(|(f, _)| (f.start_byte(), std::cmp::Reverse(f.end_byte())));

        let mut functions = Vec::new();
        let mut active_ranges: Vec<usize> = Vec::new();

        for (func_node, name_node) in func_pairs {
            let name = self.parser.node_text(name_node);
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
            let class_name = self.parser.find_enclosing_class(func_node);
            functions.push(FunctionInfo {
                name,
                location: self.parser.node_location(func_node),
                body: if include_body {
                    self.parser.node_text_utf8(func_node)
                } else {
                    String::new()
                },
                is_method: class_name.is_some(),
                class_name,
            });
        }
        functions
    }

    fn functions(&mut self) -> &[FunctionInfo] {
        if self.cache.functions().is_none() {
            self.cache.set_functions(self.build_functions(false));
        }
        self.cache.functions().unwrap_or(&[])
    }

    fn functions_with_bodies(&mut self) -> &[FunctionInfo] {
        if self.cache.functions_with_bodies().is_none() {
            self.cache
                .set_functions_with_bodies(self.build_functions(true));
        }
        self.cache.functions_with_bodies().unwrap_or(&[])
    }

    fn classes(&mut self) -> &[ClassInfo] {
        if self.cache.classes().is_none() {
            let mut methods_by_class: HashMap<String, Vec<String>> = HashMap::new();
            if self.parser.language == "go" {
                for func in self.functions() {
                    if let Some(ref cn) = func.class_name {
                        methods_by_class
                            .entry(cn.clone())
                            .or_default()
                            .push(func.name.clone());
                    }
                }
                for methods in methods_by_class.values_mut() {
                    methods.sort_unstable();
                    methods.dedup();
                }
            }

            let matches =
                self.parser
                    .run_query_pairs(self.parser.lang_info.class_query, "class", "name");
            let mut class_pairs = matches;
            class_pairs.sort_by_key(|(c, _)| (c.start_byte(), std::cmp::Reverse(c.end_byte())));

            let mut classes = Vec::new();
            let mut active_ranges: Vec<usize> = Vec::new();

            for (class_node, name_node) in class_pairs {
                let name = self.parser.node_text(name_node);
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
                if is_nested && self.parser.language != "java" {
                    continue;
                }
                active_ranges.push(end);

                let mut methods = self.parser.extract_methods_from_class(class_node);
                if self.parser.language == "go" {
                    if let Some(go_methods) = methods_by_class.get(&name) {
                        methods.extend(go_methods.iter().cloned());
                        methods.sort_unstable();
                        methods.dedup();
                    }
                }
                let fields = self.parser.extract_fields_from_class(class_node);
                let super_classes = self.parser.extract_super_classes(class_node);

                classes.push(ClassInfo {
                    name,
                    location: self.parser.node_location(class_node),
                    methods,
                    fields,
                    super_classes,
                });
            }
            self.cache.set_classes(classes);
        }
        self.cache.classes().unwrap_or(&[])
    }

    fn imports(&mut self) -> &[ImportInfo] {
        if self.cache.imports().is_none() {
            let module_nodes = self
                .parser
                .run_query_capture(self.parser.lang_info.import_query, "module");

            let mut imports = Vec::new();
            for node in module_nodes {
                let text = self.parser.node_text_unquoted(node).into_owned();
                imports.push(ImportInfo {
                    module: text,
                    location: self.parser.node_location(node),
                });
            }
            self.cache.set_imports(imports);
        }
        self.cache.imports().unwrap_or(&[])
    }

    fn ensure_calls(&mut self) {
        if self.cache.has_calls() {
            return;
        }

        let call_matches = self
            .parser
            .run_call_query_matches(self.parser.lang_info.call_query);
        let mut calls = Vec::new();
        for m in &call_matches {
            let call_node = m.call;
            let caller = self.parser.find_enclosing_function(call_node);
            let caller_class_name = self.parser.find_enclosing_class(call_node);
            let mut callee = String::new();
            let mut is_method = false;
            let mut obj_name: Option<String> = None;
            let mut func_node_opt: Option<Node<'_>> = None;

            if self.parser.language == "java" {
                if call_node.kind() == "explicit_constructor_invocation" {
                    if let Some(ctor_node) = call_node.child_by_field_name("constructor") {
                        callee = self.parser.node_text(ctor_node);
                    }
                } else if call_node.kind() == "object_creation_expression" {
                    if let Some(type_node) = call_node.child_by_field_name("type") {
                        if type_node.kind() == "generic_type" {
                            for i in 0..type_node.child_count() {
                                let child = type_node.child((i) as u32).unwrap();
                                if child.kind() == "type_identifier" {
                                    callee = self.parser.node_text(child);
                                    break;
                                }
                            }
                        } else {
                            callee = self.parser.node_text(type_node);
                        }
                    }
                } else {
                    if let Some(name_node) = call_node.child_by_field_name("name") {
                        callee = self.parser.node_text(name_node);
                    }
                    if let Some(object_node) = call_node.child_by_field_name("object") {
                        is_method = true;
                        obj_name = Some(self.parser.node_text(object_node));
                    }
                }
            } else {
                if let Some(func_node) = call_node.child_by_field_name("function") {
                    func_node_opt = Some(func_node);
                    if func_node.kind() == "identifier" {
                        callee = self.parser.node_text(func_node);
                    } else if matches!(
                        func_node.kind(),
                        "attribute" | "member_expression" | "selector_expression"
                    ) {
                        is_method = true;
                        let (c, o) = self.parser.parse_attribute_node(func_node);
                        callee = c;
                        obj_name = o;
                    }
                }
                // Handle callee/method captures from the query
                if callee.is_empty() {
                    if let Some(callee_cap) = m.callee {
                        callee = self.parser.node_text(callee_cap);
                    }
                    if let Some(method_cap) = m.method {
                        callee = self.parser.node_text(method_cap);
                        is_method = true;
                        if let Some(obj_cap) = m.object {
                            obj_name = Some(self.parser.node_text(obj_cap));
                        }
                    }
                }
            }

            if !callee.is_empty() {
                let mut resolved_callees = vec![callee.clone()];
                if matches!(
                    self.parser.language.as_str(),
                    "javascript" | "typescript" | "tsx"
                ) && !is_method
                    && func_node_opt
                        .map(|n| n.kind() == "identifier")
                        .unwrap_or(false)
                {
                    let resolved = self
                        .parser
                        .resolve_js_identifier_call_targets(call_node, &callee);
                    if !resolved.is_empty() {
                        resolved_callees = resolved;
                    }
                }

                for resolved_callee in resolved_callees {
                    calls.push(CallInfo {
                        callee: resolved_callee,
                        location: self.parser.node_location(call_node),
                        caller: caller.clone(),
                        caller_class_name: caller_class_name.clone(),
                        object_name: obj_name.clone(),
                        is_method_call: is_method,
                    });
                }
            }
        }

        // Build indices
        let mut by_callee: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_caller: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, call) in calls.iter().enumerate() {
            by_callee
                .entry(call.callee.clone())
                .or_default()
                .push(index);
            if let Some(ref c) = call.caller {
                by_caller.entry(c.clone()).or_default().push(index);
            }
        }
        self.cache.set_calls(calls, by_callee, by_caller);
    }

    pub fn get_functions(&mut self) -> Vec<FunctionInfo> {
        self.functions().to_vec()
    }

    pub fn get_functions_with_bodies(&mut self) -> Vec<FunctionInfo> {
        self.functions_with_bodies().to_vec()
    }

    pub fn get_function_by_name(
        &mut self,
        name: &str,
        class_name: Option<&str>,
    ) -> Option<FunctionInfo> {
        self.functions()
            .iter()
            .find(|f| {
                f.name == name && (class_name.is_none() || f.class_name.as_deref() == class_name)
            })
            .cloned()
    }

    pub fn get_function_definition_by_name(
        &mut self,
        name: &str,
        class_name: Option<&str>,
    ) -> Option<FunctionInfo> {
        self.functions_with_bodies()
            .iter()
            .find(|f| {
                f.name == name && (class_name.is_none() || f.class_name.as_deref() == class_name)
            })
            .cloned()
    }

    pub fn get_all_functions_by_name(
        &mut self,
        name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        self.functions()
            .iter()
            .filter(|f| {
                f.name == name && (class_name.is_none() || f.class_name.as_deref() == class_name)
            })
            .cloned()
            .collect()
    }

    pub fn get_all_function_definitions_by_name(
        &mut self,
        name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        self.functions_with_bodies()
            .iter()
            .filter(|f| {
                f.name == name && (class_name.is_none() || f.class_name.as_deref() == class_name)
            })
            .cloned()
            .collect()
    }

    pub fn get_classes(&mut self) -> Vec<ClassInfo> {
        self.classes().to_vec()
    }

    pub fn get_fields(&mut self, class_name: &str) -> Vec<FieldInfo> {
        if let Some(cached) = self.cache.fields(class_name) {
            return cached.clone();
        }

        let mut fields = Vec::new();
        if self.classes().iter().any(|cls| cls.name == class_name) {
            fields.extend(self.parser.get_fields_from_class_node(class_name));
        }
        self.cache
            .insert_fields(class_name.to_string(), fields.clone());
        fields
    }

    pub fn get_calls(&mut self) -> Vec<CallInfo> {
        self.ensure_calls();
        self.cache.calls().unwrap_or(&[]).to_vec()
    }

    pub fn get_imports(&mut self) -> Vec<ImportInfo> {
        self.imports().to_vec()
    }

    pub fn get_function_callers(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<(String, usize)> {
        self.ensure_calls();
        let by_callee = self.cache.calls_by_callee().unwrap();
        let calls = self.cache.calls().unwrap();

        let mut target_function = function_name;
        let mut target_object: Option<&str> = None;
        if function_name.contains('.') {
            let parts: Vec<&str> = function_name.rsplitn(2, '.').collect();
            target_function = parts[0];
            target_object = Some(parts[1]);
        }

        let mut callers = Vec::new();
        let mut seen: HashSet<(String, usize)> = HashSet::new();
        if let Some(candidate_call_indices) = by_callee.get(target_function) {
            for &call_index in candidate_call_indices {
                let call = &calls[call_index];
                if let Some(to) = target_object {
                    if call.object_name.as_deref() != Some(to) {
                        continue;
                    }
                }
                if let Some(cn) = class_name {
                    if !self.parser.matches_call_target_class(call, cn) {
                        continue;
                    }
                }
                let caller = call
                    .caller
                    .clone()
                    .unwrap_or_else(|| "<module>".to_string());
                let line = call.location.start_line;
                let key = (caller.clone(), line);
                if seen.insert(key) {
                    callers.push((caller, line));
                }
            }
        }

        if self.parser.language == "python"
            && self.parser.is_python_property(function_name, class_name)
        {
            for (caller, line) in self.parser.find_python_property_callers(function_name) {
                let key = (caller.clone(), line);
                if seen.insert(key) {
                    callers.push((caller, line));
                }
            }
        }
        callers
    }

    pub fn get_function_callees(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<(String, usize, Option<String>)> {
        let funcs = self.get_all_functions_by_name(function_name, class_name);
        self.ensure_calls();
        let by_caller = self.cache.calls_by_caller().unwrap();
        let calls = self.cache.calls().unwrap();

        let mut callees = Vec::new();
        let mut seen: HashSet<(String, Option<String>)> = HashSet::new();

        if !funcs.is_empty() {
            for func in &funcs {
                if let Some(caller_call_indices) = by_caller.get(&func.name) {
                    for &call_index in caller_call_indices {
                        let call = &calls[call_index];
                        if call.caller_class_name != func.class_name {
                            continue;
                        }
                        let mut callee_name = call.callee.clone();
                        if let Some(ref obj) = call.object_name {
                            callee_name = format!("{}.{}", obj, callee_name);
                        }
                        let key = (callee_name.clone(), func.class_name.clone());
                        if seen.insert(key) {
                            callees.push((
                                callee_name,
                                call.location.start_line,
                                func.class_name.clone(),
                            ));
                        }
                    }
                }
            }
        } else if let Some(caller_call_indices) = by_caller.get(function_name) {
            for &call_index in caller_call_indices {
                let call = &calls[call_index];
                if class_name.is_some() && call.caller_class_name.as_deref() != class_name {
                    continue;
                }
                let mut callee_name = call.callee.clone();
                if let Some(ref obj) = call.object_name {
                    callee_name = format!("{}.{}", obj, callee_name);
                }
                let key = (callee_name.clone(), call.caller_class_name.clone());
                if seen.insert(key) {
                    callees.push((
                        callee_name,
                        call.location.start_line,
                        call.caller_class_name.clone(),
                    ));
                }
            }
        }
        callees
    }

    pub fn find_symbols(&mut self, name: &str) -> Vec<serde_json::Value> {
        let name_bytes = name.as_bytes();
        if !self
            .parser
            .source
            .windows(name_bytes.len())
            .any(|w| w == name_bytes)
        {
            return Vec::new();
        }

        let mut refs = Vec::new();
        let mut stack = vec![self.parser.tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.is_named() && self.parser.node_bytes(node) == name_bytes {
                let context = node
                    .parent()
                    .map(|p| self.parser.node_text_utf8(p))
                    .unwrap_or_default();
                refs.push(serde_json::json!({
                    "type": node.kind(),
                    "location": {
                        "file": self.parser.file_path,
                        "start_line": node.start_position().row + 1,
                        "end_line": node.end_position().row + 1,
                    },
                    "context": context,
                }));
            }
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child((i) as u32) {
                    stack.push(child);
                }
            }
        }
        refs
    }

    pub fn get_class_by_name(&mut self, class_name: &str) -> Option<ClassInfo> {
        self.classes()
            .iter()
            .find(|c| c.name == class_name)
            .cloned()
    }

    pub fn get_super_classes(&mut self, class_name: &str) -> Vec<ClassInfo> {
        let all_classes = self.classes();
        let class_map: HashMap<&str, &ClassInfo> =
            all_classes.iter().map(|c| (c.name.as_str(), c)).collect();

        let target = match class_map.get(class_name) {
            Some(c) => *c,
            None => return Vec::new(),
        };

        let mut result = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(class_name.to_string());
        let mut queue: VecDeque<&ClassInfo> = VecDeque::new();
        queue.push_back(target);

        while let Some(current) = queue.pop_front() {
            for parent_name in &current.super_classes {
                if visited.contains(parent_name) {
                    continue;
                }
                visited.insert(parent_name.clone());
                if let Some(&parent_class) = class_map.get(parent_name.as_str()) {
                    result.push(parent_class.clone());
                    queue.push_back(parent_class);
                }
            }
        }
        result
    }

    pub fn get_sub_classes(&mut self, class_name: &str) -> Vec<ClassInfo> {
        let all_classes = self.classes();
        let mut inheritance_map: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, cls) in all_classes.iter().enumerate() {
            for parent in &cls.super_classes {
                inheritance_map
                    .entry(parent.as_str())
                    .or_default()
                    .push(index);
            }
        }

        let mut result = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(class_name.to_string());
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(class_name.to_string());

        while let Some(current_name) = queue.pop_front() {
            if let Some(children) = inheritance_map.get(current_name.as_str()) {
                for &child_index in children {
                    let child = &all_classes[child_index];
                    if visited.insert(child.name.clone()) {
                        result.push(child.clone());
                        queue.push_back(child.name.clone());
                    }
                }
            }
        }
        result
    }
}
