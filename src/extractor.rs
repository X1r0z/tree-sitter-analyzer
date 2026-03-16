use std::collections::{HashMap, HashSet};

use crate::models::*;
use crate::parser::call_targets::{
    has_non_self_object_target, matches_call_target, matches_module_property_target,
    matches_property_target as call_matches_property_target, split_function_target,
    type_matches_class,
};
use crate::parser::capture;
use crate::parser::ParseContext;
use crate::utils::select_most_specific_by_line;

struct ParseCache {
    functions: Option<Vec<FunctionInfo>>,
    functions_with_bodies: Option<Vec<FunctionInfo>>,
    classes: Option<Vec<ClassInfo>>,
    calls: Option<Vec<CallInfo>>,
    calls_by_callee: Option<HashMap<String, Vec<usize>>>,
    calls_by_caller: Option<HashMap<String, Vec<usize>>>,
    imports: Option<Vec<ImportInfo>>,
    fields_by_class: HashMap<String, Vec<FieldInfo>>,
}

pub struct CodeExtractor {
    parser: ParseContext,
    cache: ParseCache,
}

impl CodeExtractor {
    pub fn new(file_path: &str) -> anyhow::Result<Self> {
        let parser = ParseContext::new(file_path)?;
        Ok(Self {
            parser,
            cache: ParseCache {
                functions: None,
                functions_with_bodies: None,
                classes: None,
                calls: None,
                calls_by_callee: None,
                calls_by_caller: None,
                imports: None,
                fields_by_class: HashMap::new(),
            },
        })
    }

    fn ensure_calls(&mut self) {
        if self.cache.calls.is_some() {
            return;
        }
        self.cache.calls = Some(self.parser.collect_calls());
    }

    fn ensure_calls_by_callee(&mut self) {
        self.ensure_calls();
        if self.cache.calls_by_callee.is_some() {
            return;
        }
        let mut by_callee: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, call) in self
            .cache
            .calls
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            by_callee
                .entry(call.callee.clone())
                .or_default()
                .push(index);
        }
        self.cache.calls_by_callee = Some(by_callee);
    }

    fn ensure_calls_by_caller(&mut self) {
        self.ensure_calls();
        if self.cache.calls_by_caller.is_some() {
            return;
        }
        let mut by_caller: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, call) in self
            .cache
            .calls
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            if let Some(ref caller) = call.caller {
                by_caller.entry(caller.clone()).or_default().push(index);
            }
        }
        self.cache.calls_by_caller = Some(by_caller);
    }

    fn enclosing_function_info_at_line(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
        line: usize,
    ) -> Option<FunctionInfo> {
        let matches_target = |function: &FunctionInfo| {
            function.name == function_name
                && match class_name {
                    Some(class_name) => function.class_name.as_deref() == Some(class_name),
                    None => true,
                }
        };
        select_most_specific_by_line(self.collect_functions(), line, |function| {
            (function.location.start_line, function.location.end_line)
        })
        .filter(&matches_target)
        .or_else(|| {
            select_most_specific_by_line(
                self.collect_functions().into_iter().filter(matches_target),
                line,
                |function| (function.location.start_line, function.location.end_line),
            )
        })
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

    fn matches_property_target(
        &mut self,
        caller_name: Option<&str>,
        caller_class_name: Option<&str>,
        object_name: Option<&str>,
        object_type: Option<&str>,
        line: usize,
        class_name: &str,
    ) -> bool {
        if caller_name == Some("<module>") {
            return matches_module_property_target(object_name, object_type, class_name);
        }
        let field_infos = caller_class_name
            .map(|caller_class_name| self.collect_fields(caller_class_name))
            .unwrap_or_default();
        let caller_params = caller_name
            .and_then(|caller_name| {
                self.enclosing_function_info_at_line(caller_name, caller_class_name, line)
            })
            .map(|caller| caller.params)
            .unwrap_or_default();
        call_matches_property_target(
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

    pub fn collect_functions(&mut self) -> Vec<FunctionInfo> {
        if self.cache.functions.is_none() {
            self.cache.functions = Some(self.parser.collect_functions(false));
        }
        self.cache.functions.as_deref().unwrap_or(&[]).to_vec()
    }

    pub fn collect_functions_with_bodies(&mut self) -> Vec<FunctionInfo> {
        if self.cache.functions_with_bodies.is_none() {
            self.cache.functions_with_bodies = Some(self.parser.collect_functions(true));
        }
        self.cache
            .functions_with_bodies
            .as_deref()
            .unwrap_or(&[])
            .to_vec()
    }

    pub fn has_function_named(&self, name: &str, class_name: Option<&str>) -> bool {
        capture::has_function_capture_named(&self.parser, name, class_name)
    }

    pub fn find_function_definitions(
        &mut self,
        name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        self.collect_functions_with_bodies()
            .iter()
            .filter(|f| {
                f.name == name && (class_name.is_none() || f.class_name.as_deref() == class_name)
            })
            .cloned()
            .collect()
    }

    pub fn collect_classes(&mut self) -> Vec<ClassInfo> {
        if self.cache.classes.is_none() {
            self.cache.classes = Some(self.parser.collect_classes());
        }
        self.cache.classes.as_deref().unwrap_or(&[]).to_vec()
    }

    pub fn collect_fields(&mut self, class_name: &str) -> Vec<FieldInfo> {
        if let Some(cached) = self.cache.fields_by_class.get(class_name) {
            return cached.clone();
        }

        let mut fields = Vec::new();
        if self
            .collect_classes()
            .iter()
            .any(|cls| cls.name == class_name)
        {
            fields.extend(self.parser.collect_field_infos_for_class(class_name));
        }
        self.cache
            .fields_by_class
            .insert(class_name.to_string(), fields.clone());
        fields
    }

    pub fn collect_calls(&mut self) -> Vec<CallInfo> {
        self.ensure_calls();
        self.cache.calls.as_deref().unwrap_or(&[]).to_vec()
    }

    pub fn collect_imports(&mut self) -> Vec<ImportInfo> {
        if self.cache.imports.is_none() {
            self.cache.imports = Some(self.parser.collect_imports());
        }
        self.cache.imports.as_deref().unwrap_or(&[]).to_vec()
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
        let refs = self.parser.collect_refs();
        let python_properties = self.parser.collect_python_properties();
        let python_property_callers = self.parser.collect_python_property_callers(None);

        AnalyzerSnapshot {
            functions,
            classes,
            fields,
            calls,
            imports,
            annotations,
            refs,
            python_properties,
            python_property_callers,
        }
    }

    pub fn find_function_callers(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
        allow_unique_method_target: bool,
    ) -> Vec<(String, usize)> {
        self.ensure_calls_by_callee();

        let (target_function, target_object) = split_function_target(function_name);
        let candidate_call_indices = self
            .cache
            .calls_by_callee
            .as_ref()
            .and_then(|by_callee| by_callee.get(target_function))
            .cloned()
            .unwrap_or_default();
        let candidate_calls: Vec<CallInfo> = candidate_call_indices
            .into_iter()
            .filter_map(|index| {
                self.cache
                    .calls
                    .as_ref()
                    .and_then(|calls| calls.get(index))
                    .cloned()
            })
            .collect();

        let mut callers = Vec::new();
        let mut seen: HashSet<(String, usize)> = HashSet::new();
        for call in candidate_calls {
            if let Some(to) = target_object {
                if call.object_name.as_deref() != Some(to) {
                    continue;
                }
            }
            if let Some(cn) = class_name {
                if !(self.call_matches_class_target(&call, cn)
                    || allow_unique_method_target
                        && has_non_self_object_target(call.object_name.as_deref()))
                {
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

        if self
            .parser
            .has_python_property_definition(function_name, class_name)
        {
            for caller in self
                .parser
                .collect_python_property_callers(Some(function_name))
            {
                if let Some(class_name) = class_name {
                    if !self.matches_property_target(
                        Some(&caller.caller),
                        caller.caller_class_name.as_deref(),
                        caller.object_name.as_deref(),
                        caller.object_type.as_deref(),
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
        let candidate_calls: Vec<CallInfo> = self
            .cache
            .calls_by_caller
            .as_ref()
            .and_then(|by_caller| by_caller.get(function_name))
            .into_iter()
            .flatten()
            .filter_map(|&index| {
                self.cache
                    .calls
                    .as_ref()
                    .and_then(|calls| calls.get(index))
                    .cloned()
            })
            .collect();

        let mut callees = Vec::new();
        let mut seen: HashSet<(String, usize, Option<String>)> = HashSet::new();
        for call in candidate_calls {
            let Some(caller) = self.enclosing_function_info_at_line(
                function_name,
                class_name,
                call.location.start_line,
            ) else {
                continue;
            };
            let mut callee_name = call.callee.clone();
            if let Some(ref obj) = call.object_name {
                callee_name = format!("{}.{}", obj, callee_name);
            }
            let key = (
                callee_name.clone(),
                call.location.start_line,
                caller.class_name.clone(),
            );
            if seen.insert(key) {
                callees.push((callee_name, call.location.start_line, caller.class_name));
            }
        }
        callees
    }

    pub fn collect_annotations(&self) -> Vec<AnnotationInfo> {
        self.parser.collect_annotations()
    }

    pub fn find_refs(&mut self, name: &str) -> Vec<RefInfo> {
        self.parser.find_refs(name, true)
    }

    pub fn hydrate_refs(&mut self, candidates: &[RefInfo]) -> Vec<RefInfo> {
        self.parser.hydrate_refs(candidates)
    }

    pub fn find_class(&mut self, class_name: &str) -> Option<ClassInfo> {
        self.collect_classes()
            .iter()
            .find(|c| c.name == class_name)
            .cloned()
    }
}
