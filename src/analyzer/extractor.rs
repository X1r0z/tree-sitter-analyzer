use std::collections::{HashMap, HashSet};

use super::cache::AnalyzerCache;
use crate::analyzer::call_targets::{
    matches_call_target, split_function_target, type_matches_class,
};
use crate::models::*;
use crate::parser::query;
use crate::parser::{languages::python, ParseContext};

pub struct CodeExtractor {
    parser: ParseContext,
    cache: AnalyzerCache,
}

impl CodeExtractor {
    pub fn new(file_path: &str) -> anyhow::Result<Self> {
        let parser = ParseContext::new(file_path)?;
        Ok(Self {
            parser,
            cache: AnalyzerCache::new(),
        })
    }

    fn cached_functions(&mut self) -> &[FunctionInfo] {
        if self.cache.functions().is_none() {
            self.cache
                .set_functions(self.parser.collect_functions(false));
        }
        self.cache.functions().unwrap_or(&[])
    }

    fn cached_functions_with_bodies(&mut self) -> &[FunctionInfo] {
        if self.cache.functions_with_bodies().is_none() {
            self.cache
                .set_functions_with_bodies(self.parser.collect_functions(true));
        }
        self.cache.functions_with_bodies().unwrap_or(&[])
    }

    fn cached_classes(&mut self) -> &[ClassInfo] {
        if self.cache.classes().is_none() {
            self.cache.set_classes(self.parser.collect_classes());
        }
        self.cache.classes().unwrap_or(&[])
    }

    fn cached_imports(&mut self) -> &[ImportInfo] {
        if self.cache.imports().is_none() {
            self.cache.set_imports(self.parser.collect_imports());
        }
        self.cache.imports().unwrap_or(&[])
    }

    fn ensure_calls(&mut self) {
        if self.cache.has_calls() {
            return;
        }
        self.cache.set_calls(self.parser.collect_calls());
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
        let (properties, callers) = python::collect_python_property_indexes(&self.parser);
        self.cache.set_python_properties(properties, callers);
    }

    fn enclosing_function_info_at_line(
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

    fn matches_class_target(
        &mut self,
        caller_name: Option<&str>,
        caller_class_name: Option<&str>,
        object_name: Option<&str>,
        line: usize,
        class_name: &str,
    ) -> bool {
        let field_infos = caller_class_name
            .map(|caller_class_name| self.collect_fields(caller_class_name))
            .unwrap_or_default();
        let caller_params = caller_name
            .and_then(|caller_name| {
                self.enclosing_function_info_at_line(caller_name, caller_class_name, line)
            })
            .map(|caller| caller.params)
            .unwrap_or_default();
        matches_call_target(
            caller_class_name,
            object_name,
            class_name,
            |attr_name, target_class_name| {
                field_infos.iter().any(|field| {
                    field.name == attr_name
                        && type_matches_class(field.field_type.as_deref(), target_class_name)
                })
            },
            |attr_name, target_class_name| {
                caller_params.iter().any(|param| {
                    param.name == attr_name
                        && type_matches_class(param.param_type.as_deref(), target_class_name)
                })
            },
        )
    }

    fn call_matches_class_target(&mut self, call: &CallInfo, class_name: &str) -> bool {
        self.matches_class_target(
            call.caller.as_deref(),
            call.caller_class_name.as_deref(),
            call.object_name.as_deref(),
            call.location.start_line,
            class_name,
        )
    }

    pub fn collect_functions(&mut self) -> Vec<FunctionInfo> {
        self.cached_functions().to_vec()
    }

    pub fn collect_functions_with_bodies(&mut self) -> Vec<FunctionInfo> {
        self.cached_functions_with_bodies().to_vec()
    }

    pub fn has_function_named(&self, name: &str, class_name: Option<&str>) -> bool {
        query::has_function_capture_named(&self.parser, name, class_name)
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

    pub fn collect_classes(&mut self) -> Vec<ClassInfo> {
        self.cached_classes().to_vec()
    }

    pub fn collect_fields(&mut self, class_name: &str) -> Vec<FieldInfo> {
        if let Some(cached) = self.cache.fields(class_name) {
            return cached.clone();
        }

        let mut fields = Vec::new();
        if self
            .cached_classes()
            .iter()
            .any(|cls| cls.name == class_name)
        {
            fields.extend(self.parser.collect_field_infos_for_class(class_name));
        }
        self.cache
            .insert_fields(class_name.to_string(), fields.clone());
        fields
    }

    pub fn collect_calls(&mut self) -> Vec<CallInfo> {
        self.ensure_calls();
        self.cache.calls().unwrap_or(&[]).to_vec()
    }

    pub fn collect_imports(&mut self) -> Vec<ImportInfo> {
        self.cached_imports().to_vec()
    }

    pub fn snapshot_for_index(&mut self) -> AnalyzerSnapshot {
        let functions = self.collect_functions();
        let classes = self.collect_classes();

        let mut fields = Vec::new();
        for class in &classes {
            fields.extend(self.collect_fields(&class.name));
        }

        let calls = self.collect_calls();
        let imports = self.collect_imports();
        let annotations = self.collect_annotations();
        let symbols = self.parser.collect_symbols();

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
                if !self.call_matches_class_target(&call, cn) {
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
                    if !self.matches_class_target(
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

    pub fn collect_annotations(&self) -> Vec<AnnotationInfo> {
        self.parser.collect_annotations()
    }

    pub fn find_symbols(&mut self, name: &str) -> Vec<SymbolRefInfo> {
        self.parser.find_symbols(name, true)
    }

    pub fn hydrate_symbols(&mut self, candidates: &[SymbolRefInfo]) -> Vec<SymbolRefInfo> {
        self.parser.hydrate_symbols(candidates)
    }

    pub fn find_class(&mut self, class_name: &str) -> Option<ClassInfo> {
        self.cached_classes()
            .iter()
            .find(|c| c.name == class_name)
            .cloned()
    }
}
