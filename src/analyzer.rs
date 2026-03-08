use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::cache::AnalyzerCache;
use crate::languages::QueryKind;
use crate::models::*;
use crate::parser::BaseParser;
use crate::symbols;
use crate::utils::{extract_instance_attr, split_function_target, type_matches_class};

pub struct CodeAnalyzer {
    parser: BaseParser,
    cache: AnalyzerCache,
}

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
                .query_capture_pairs(QueryKind::Function, "function", "name");
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
            let class_name = self.parser.find_enclosing_class_name(func_node);
            functions.push(FunctionInfo {
                name,
                location: self.parser.node_location(func_node),
                body: if include_body {
                    self.parser.node_text(func_node)
                } else {
                    String::new()
                },
                class_name,
                params: self.parser.extract_function_params(func_node),
            });
        }
        functions
    }

    fn cached_functions(&mut self) -> &[FunctionInfo] {
        if self.cache.functions().is_none() {
            self.cache.set_functions(self.build_functions(false));
        }
        self.cache.functions().unwrap_or(&[])
    }

    fn cached_functions_with_bodies(&mut self) -> &[FunctionInfo] {
        if self.cache.functions_with_bodies().is_none() {
            self.cache
                .set_functions_with_bodies(self.build_functions(true));
        }
        self.cache.functions_with_bodies().unwrap_or(&[])
    }

    fn cached_classes(&mut self) -> &[ClassInfo] {
        if self.cache.classes().is_none() {
            let mut methods_by_class: HashMap<String, Vec<String>> = HashMap::new();
            if self.parser.language == "go" {
                for func in self.cached_functions() {
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

            let matches = self
                .parser
                .query_capture_pairs(QueryKind::Class, "class", "name");
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

                let mut methods = self.parser.extract_method_names_from_class(class_node);
                if self.parser.language == "go" {
                    if let Some(go_methods) = methods_by_class.get(&name) {
                        methods.extend(go_methods.iter().cloned());
                        methods.sort_unstable();
                        methods.dedup();
                    }
                }
                let fields = self.parser.extract_field_names_from_class(class_node);
                let super_classes = self.parser.extract_super_class_names(class_node);

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

    fn cached_imports(&mut self) -> &[ImportInfo] {
        if self.cache.imports().is_none() {
            let module_nodes = self
                .parser
                .query_capture_nodes_for(QueryKind::Import, "module");

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

        let mut call_matches = self.parser.query_call_matches_for(QueryKind::Call);
        let is_js_like = matches!(
            self.parser.language.as_str(),
            "javascript" | "typescript" | "tsx"
        );
        if is_js_like {
            call_matches.sort_by_key(|m| m.call.start_byte());
        }
        let mut calls = Vec::new();
        for m in &call_matches {
            let call_node = m.call;
            let (caller, caller_class_name, enclosing_function_node) =
                self.parser.find_enclosing_context(call_node);
            let mut callee = String::new();
            let mut is_method = false;
            let mut obj_name: Option<String> = None;
            let mut callee_function_node_opt: Option<Node<'_>> = None;

            if self.parser.language == "java" {
                if call_node.kind() == "explicit_constructor_invocation" {
                    if let Some(ctor_node) = call_node.child_by_field_name("constructor") {
                        callee = self.parser.node_text(ctor_node);
                    }
                } else if call_node.kind() == "object_creation_expression" {
                    if let Some(type_node) = call_node.child_by_field_name("type") {
                        if type_node.kind() == "generic_type" {
                            for i in 0..type_node.named_child_count() {
                                let child = type_node.named_child(i as u32).unwrap();
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
                    callee_function_node_opt = Some(func_node);
                    if func_node.kind() == "identifier" {
                        callee = self.parser.node_text(func_node);
                    } else if matches!(
                        func_node.kind(),
                        "attribute" | "member_expression" | "selector_expression"
                    ) {
                        is_method = true;
                        let (c, o) = self.parser.extract_attribute_parts(func_node);
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
                let call_location = self.parser.node_location(call_node);
                let mut pushed_resolved_calls = false;
                if is_js_like
                    && !is_method
                    && callee_function_node_opt
                        .map(|n| n.kind() == "identifier")
                        .unwrap_or(false)
                    && enclosing_function_node.is_some()
                {
                    let resolved = self
                        .parser
                        .resolve_js_call_targets_for_identifier(call_node, &callee);
                    if !resolved.is_empty() {
                        for resolved_callee in resolved {
                            calls.push(CallInfo {
                                callee: resolved_callee,
                                location: call_location.clone(),
                                caller: caller.clone(),
                                caller_class_name: caller_class_name.clone(),
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
                        caller,
                        caller_class_name,
                        object_name: obj_name,
                    });
                }
            }
        }

        self.cache.set_calls(calls);
    }

    fn ensure_calls_by_callee(&mut self) {
        self.ensure_calls();
        if self.cache.calls_by_callee().is_some() {
            return;
        }
        let mut by_callee: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, call) in self.cache.calls().unwrap_or(&[]).iter().enumerate() {
            by_callee
                .entry(call.callee.clone())
                .or_default()
                .push(index);
        }
        self.cache.set_calls_by_callee(by_callee);
    }

    fn ensure_calls_by_caller(&mut self) {
        self.ensure_calls();
        if self.cache.calls_by_caller().is_some() {
            return;
        }
        let mut by_caller: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, call) in self.cache.calls().unwrap_or(&[]).iter().enumerate() {
            if let Some(ref caller) = call.caller {
                by_caller.entry(caller.clone()).or_default().push(index);
            }
        }
        self.cache.set_calls_by_caller(by_caller);
    }

    fn ensure_python_property_index(&mut self) {
        if self.parser.language != "python" || self.cache.python_properties().is_some() {
            return;
        }
        let (properties, callers) = self.parser.build_python_property_indexes();
        self.cache.set_python_properties(properties, callers);
    }

    fn find_enclosing_function_info(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
        line: usize,
    ) -> Option<FunctionInfo> {
        self.cached_functions()
            .iter()
            .find(|function| {
                function.name == function_name
                    && function.class_name.as_deref() == class_name
                    && function.location.start_line <= line
                    && line <= function.location.end_line
            })
            .cloned()
    }

    fn matches_target_class(
        &mut self,
        caller_name: Option<&str>,
        caller_class_name: Option<&str>,
        object_name: Option<&str>,
        line: usize,
        class_name: &str,
    ) -> bool {
        if object_name == Some(class_name) {
            return true;
        }
        if object_name.is_none() {
            return caller_class_name == Some(class_name);
        }
        if matches!(object_name, Some("self") | Some("this") | Some("cls")) {
            return caller_class_name == Some(class_name);
        }

        let Some(object_name) = object_name else {
            return false;
        };

        if object_name
            .split('(')
            .next()
            .and_then(|head| head.rsplit('.').next())
            .is_some_and(|name| name == class_name)
        {
            return true;
        }

        let Some(attr_name) = extract_instance_attr(object_name) else {
            return false;
        };

        if let Some(caller_class_name) = caller_class_name {
            for field in self.fields(caller_class_name) {
                if field.name != attr_name {
                    continue;
                }
                if type_matches_class(field.field_type.as_deref(), class_name) {
                    return true;
                }
            }
        }

        let Some(caller_name) = caller_name else {
            return false;
        };
        let Some(caller) = self.find_enclosing_function_info(caller_name, caller_class_name, line)
        else {
            return false;
        };

        caller.params.iter().any(|param| {
            param.name == attr_name && type_matches_class(param.param_type.as_deref(), class_name)
        })
    }

    fn matches_call_target_class(&mut self, call: &CallInfo, class_name: &str) -> bool {
        self.matches_target_class(
            call.caller.as_deref(),
            call.caller_class_name.as_deref(),
            call.object_name.as_deref(),
            call.location.start_line,
            class_name,
        )
    }

    pub fn functions(&mut self) -> Vec<FunctionInfo> {
        self.cached_functions().to_vec()
    }

    pub fn functions_with_bodies(&mut self) -> Vec<FunctionInfo> {
        self.cached_functions_with_bodies().to_vec()
    }

    pub fn has_function_named(&self, name: &str, class_name: Option<&str>) -> bool {
        self.parser.has_function_named(name, class_name)
    }

    pub fn find_function_definitions(
        &mut self,
        name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        self.cached_functions_with_bodies()
            .iter()
            .filter(|f| {
                f.name == name && (class_name.is_none() || f.class_name.as_deref() == class_name)
            })
            .cloned()
            .collect()
    }

    pub fn classes(&mut self) -> Vec<ClassInfo> {
        self.cached_classes().to_vec()
    }

    pub fn fields(&mut self, class_name: &str) -> Vec<FieldInfo> {
        if let Some(cached) = self.cache.fields(class_name) {
            return cached.clone();
        }

        let mut fields = Vec::new();
        if self
            .cached_classes()
            .iter()
            .any(|cls| cls.name == class_name)
        {
            fields.extend(self.parser.find_field_infos_by_class_name(class_name));
        }
        self.cache
            .insert_fields(class_name.to_string(), fields.clone());
        fields
    }

    pub fn calls(&mut self) -> Vec<CallInfo> {
        self.ensure_calls();
        self.cache.calls().unwrap_or(&[]).to_vec()
    }

    pub fn imports(&mut self) -> Vec<ImportInfo> {
        self.cached_imports().to_vec()
    }

    pub fn snapshot_for_index(&mut self) -> AnalyzerSnapshot {
        let functions = self.functions();
        let classes = self.classes();

        let mut fields = Vec::new();
        for class in &classes {
            fields.extend(self.fields(&class.name));
        }

        let calls = self.calls();
        let imports = self.imports();
        let annotations = self.annotations();
        let symbols = symbols::collect_all(&self.parser);

        let mut python_properties = Vec::new();
        let mut python_property_callers = Vec::new();
        if self.parser.language == "python" {
            self.ensure_python_property_index();
            if let Some(properties) = self.cache.python_properties() {
                python_properties = properties
                    .iter()
                    .map(|(name, class_name)| PythonPropertyInfo {
                        name: name.clone(),
                        class_name: class_name.clone(),
                    })
                    .collect();
                python_properties.sort_by(|a, b| {
                    a.name
                        .cmp(&b.name)
                        .then_with(|| a.class_name.cmp(&b.class_name))
                });
            }
            if let Some(callers) = self.cache.python_property_callers() {
                for (property_name, entries) in callers {
                    for caller in entries {
                        python_property_callers.push(PythonPropertyCallerInfo {
                            property_name: property_name.clone(),
                            file: caller.file.clone(),
                            caller: caller.caller.clone(),
                            caller_class_name: caller.caller_class_name.clone(),
                            object_name: caller.object_name.clone(),
                            line: caller.line,
                        });
                    }
                }
                python_property_callers.sort_by(|a, b| {
                    a.property_name
                        .cmp(&b.property_name)
                        .then_with(|| a.caller.cmp(&b.caller))
                        .then_with(|| a.line.cmp(&b.line))
                });
            }
        }

        AnalyzerSnapshot {
            functions,
            classes,
            fields,
            calls,
            imports,
            annotations,
            symbols,
            python_properties,
            python_property_callers,
        }
    }

    pub fn find_function_callers(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<(String, usize)> {
        self.ensure_calls_by_callee();
        let by_callee = self.cache.calls_by_callee().unwrap();
        let calls = self.cache.calls().unwrap().to_vec();

        let (target_function, target_object) = split_function_target(function_name);

        let mut callers = Vec::new();
        let mut seen: HashSet<(String, usize)> = HashSet::new();
        let candidate_call_indices = by_callee.get(target_function).cloned().unwrap_or_default();
        for call_index in candidate_call_indices {
            let call = calls[call_index].clone();
            if let Some(to) = target_object {
                if call.object_name.as_deref() != Some(to) {
                    continue;
                }
            }
            if let Some(cn) = class_name {
                if !self.matches_call_target_class(&call, cn) {
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

        if self.parser.language == "python" {
            self.ensure_python_property_index();
        }
        if self.parser.language == "python"
            && self.cache.python_properties().is_some_and(|properties| {
                properties.contains(&(function_name.to_string(), class_name.map(str::to_string)))
            })
        {
            for caller in self
                .cache
                .python_property_callers()
                .and_then(|callers| callers.get(function_name))
                .cloned()
                .unwrap_or_default()
            {
                if let Some(class_name) = class_name {
                    if !self.matches_target_class(
                        Some(&caller.caller),
                        caller.caller_class_name.as_deref(),
                        caller.object_name.as_deref(),
                        caller.line,
                        class_name,
                    ) {
                        continue;
                    }
                }
                let key = (caller.caller.clone(), caller.line);
                if seen.insert(key) {
                    callers.push((caller.caller, caller.line));
                }
            }
        }
        callers
    }

    pub fn find_function_callees(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<(String, usize, Option<String>)> {
        self.ensure_calls_by_caller();
        let by_caller = self.cache.calls_by_caller().unwrap();
        let calls = self.cache.calls().unwrap();

        let mut callees = Vec::new();
        let mut seen: HashSet<(String, Option<String>)> = HashSet::new();
        if let Some(caller_call_indices) = by_caller.get(function_name) {
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

    pub fn annotations(&self) -> Vec<AnnotationInfo> {
        self.parser.extract_annotations()
    }

    pub fn find_symbols(&mut self, name: &str) -> Vec<SymbolRefInfo> {
        symbols::find(&self.parser, name, true)
    }

    pub fn hydrate_symbol_contexts(&mut self, candidates: &[SymbolRefInfo]) -> Vec<SymbolRefInfo> {
        symbols::hydrate(&self.parser, candidates)
    }

    pub fn class_named(&mut self, class_name: &str) -> Option<ClassInfo> {
        self.cached_classes()
            .iter()
            .find(|c| c.name == class_name)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::CodeAnalyzer;

    const PYTHON_PROPERTY_COLLISION: &str = r#"class Alpha:
    @property
    def value(self):
        return 1


class Beta:
    @property
    def value(self):
        return 2


def read_alpha(obj: Alpha):
    return obj.value


def read_beta(obj: Beta):
    return obj.value
"#;

    #[test]
    fn python_property_callers_are_disambiguated_by_receiver_type() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let file = temp.path().join("sample.py");
        fs::write(&file, PYTHON_PROPERTY_COLLISION)?;

        let mut analyzer = CodeAnalyzer::new(file.to_str().unwrap())?;

        let alpha_callers = analyzer.find_function_callers("value", Some("Alpha"));
        assert_eq!(alpha_callers, vec![("read_alpha".to_string(), 14)]);

        let beta_callers = analyzer.find_function_callers("value", Some("Beta"));
        assert_eq!(beta_callers, vec![("read_beta".to_string(), 18)]);

        Ok(())
    }
}
