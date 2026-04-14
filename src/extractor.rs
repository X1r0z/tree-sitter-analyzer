use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::models::*;
use crate::parser::call_targets::{
    has_non_self_object_target, matches_call_target, matches_module_property_target,
    matches_property_target as call_matches_property_target, split_function_target,
    type_matches_class,
};
use crate::parser::languages::python;
use crate::parser::ParseContext;
use crate::traversal::collect_reachable_bfs;
use crate::utils::select_most_specific_by_line;

struct ParseCache {
    functions: Option<Vec<FunctionInfo>>,
    functions_with_bodies: Option<Vec<FunctionInfo>>,
    classes: Option<Vec<ClassInfo>>,
    snapshot_fields: Option<Vec<FieldInfo>>,
    calls: Option<Vec<CallInfo>>,
    calls_by_callee: Option<HashMap<String, Vec<usize>>>,
    calls_by_caller: Option<HashMap<String, Vec<usize>>>,
    imports: Option<Vec<ImportInfo>>,
    fields_by_class: HashMap<String, Vec<FieldInfo>>,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct SnapshotBuildTimings {
    pub(crate) functions: Duration,
    pub(crate) classes_fields: Duration,
    pub(crate) calls: Duration,
    pub(crate) call_capture_query: Duration,
    pub(crate) call_enclosing: Duration,
    pub(crate) call_resolve: Duration,
    pub(crate) call_include_filter: Duration,
    pub(crate) call_resolve_targets: Duration,
    pub(crate) imports: Duration,
    pub(crate) annotations: Duration,
    pub(crate) refs: Duration,
    pub(crate) python_properties: Duration,
}

pub struct CodeExtractor {
    parser: ParseContext,
    cache: ParseCache,
}

impl CodeExtractor {
    pub fn new(file_path: &str) -> anyhow::Result<Self> {
        let parser = ParseContext::new(file_path)?;
        Ok(Self::from_parser(parser))
    }

    pub fn from_source(file_path: &str, source: Vec<u8>) -> anyhow::Result<Self> {
        let parser = ParseContext::from_source(file_path, source)?;
        Ok(Self::from_parser(parser))
    }

    fn from_parser(parser: ParseContext) -> Self {
        Self {
            parser,
            cache: ParseCache {
                functions: None,
                functions_with_bodies: None,
                classes: None,
                snapshot_fields: None,
                calls: None,
                calls_by_callee: None,
                calls_by_caller: None,
                imports: None,
                fields_by_class: HashMap::new(),
            },
        }
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
        if self.cache.functions.is_none() {
            let mut functions = self.parser.collect_functions(false);
            functions
                .sort_by_key(|function| (function.location.start_line, function.location.end_line));
            self.cache.functions = Some(functions);
        }
        let functions = self.cache.functions.as_deref().unwrap_or(&[]);
        select_most_specific_by_line(functions, line, |function| {
            (function.location.start_line, function.location.end_line)
        })
        .filter(|function| matches_target(function))
        .cloned()
        .or_else(|| {
            let filtered: Vec<_> = functions
                .iter()
                .filter(|function| matches_target(function))
                .cloned()
                .collect();
            select_most_specific_by_line(&filtered, line, |function| {
                (function.location.start_line, function.location.end_line)
            })
            .cloned()
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
        if self.matches_super_call_target(
            call.caller_class_name.as_deref(),
            call.object_name.as_deref(),
            class_name,
        ) {
            return true;
        }
        self.matches_class_target(
            call.caller.as_deref(),
            call.caller_class_name.as_deref(),
            call.object_name.as_deref(),
            call.location.start_line,
            class_name,
        )
    }

    fn matches_super_call_target(
        &mut self,
        caller_class_name: Option<&str>,
        object_name: Option<&str>,
        class_name: &str,
    ) -> bool {
        if object_name != Some("super()") {
            return false;
        }
        let Some(caller_class_name) = caller_class_name else {
            return false;
        };
        self.parents(caller_class_name)
            .iter()
            .any(|super_class| super_class == class_name)
    }

    fn parents(&mut self, class_name: &str) -> Vec<String> {
        self.collect_classes()
            .into_iter()
            .find(|class| class.name == class_name)
            .map(|class| class.super_classes)
            .unwrap_or_default()
    }

    fn resolve_super_call_targets(&mut self, caller: &FunctionInfo, callee: &str) -> Vec<String> {
        let Some(caller_class_name) = caller.class_name.as_deref() else {
            return Vec::new();
        };
        let parents = self.parents(caller_class_name);
        if parents.is_empty() {
            return Vec::new();
        }

        let mut targets: Vec<_> = self
            .find_function_signatures(callee, None)
            .into_iter()
            .filter_map(|function| {
                let class_name = function.class_name?;
                parents
                    .iter()
                    .any(|parent| parent == &class_name)
                    .then(|| format!("{}.{}", class_name, function.name))
            })
            .collect();
        targets.sort();
        targets.dedup();
        targets
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
        if matches!(object_name, Some("self") | Some("cls")) {
            return caller_class_name.is_some_and(|caller_class_name| {
                caller_class_name == class_name
                    || collect_reachable_bfs(
                        [caller_class_name.to_string()],
                        [caller_class_name.to_string()],
                        |current| self.parents(current),
                        Clone::clone,
                        Clone::clone,
                    )
                    .into_iter()
                    .any(|ancestor| ancestor == class_name)
            });
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
            let mut functions = self.parser.collect_functions(false);
            functions
                .sort_by_key(|function| (function.location.start_line, function.location.end_line));
            self.cache.functions = Some(functions);
        }
        self.cache.functions.as_deref().unwrap_or(&[]).to_vec()
    }

    pub fn collect_functions_with_bodies(&mut self) -> Vec<FunctionInfo> {
        if self.cache.functions_with_bodies.is_none() {
            let mut functions = self.parser.collect_functions(true);
            functions
                .sort_by_key(|function| (function.location.start_line, function.location.end_line));
            self.cache.functions_with_bodies = Some(functions);
        }
        self.cache
            .functions_with_bodies
            .as_deref()
            .unwrap_or(&[])
            .to_vec()
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

    pub fn find_function_signatures(
        &mut self,
        name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        self.collect_functions()
            .iter()
            .filter(|f| {
                f.name == name && (class_name.is_none() || f.class_name.as_deref() == class_name)
            })
            .cloned()
            .collect()
    }

    pub fn collect_classes(&mut self) -> Vec<ClassInfo> {
        self.ensure_class_snapshot();
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

    pub fn build_snapshot(&mut self) -> FileSnapshot {
        self.build_snapshot_with_timings().0
    }

    pub(crate) fn build_snapshot_with_timings(&mut self) -> (FileSnapshot, SnapshotBuildTimings) {
        let mut timings = SnapshotBuildTimings::default();

        let started = Instant::now();
        let functions = self.collect_functions();
        timings.functions = started.elapsed();

        let started = Instant::now();
        self.ensure_class_snapshot();
        let classes = self.cache.classes.as_deref().unwrap_or(&[]).to_vec();
        let fields = self
            .cache
            .snapshot_fields
            .as_deref()
            .unwrap_or(&[])
            .to_vec();
        timings.classes_fields = started.elapsed();

        let started = Instant::now();
        let (calls, call_timings) = if self.cache.calls.is_some() {
            (
                self.cache.calls.as_deref().unwrap_or(&[]).to_vec(),
                crate::parser::CallCollectionTimings::default(),
            )
        } else {
            self.parser.collect_calls_with_timings()
        };
        self.cache.calls = Some(calls.clone());
        timings.calls = started.elapsed();
        timings.call_capture_query = call_timings.capture_query;
        timings.call_enclosing = call_timings.enclosing;
        timings.call_resolve = call_timings.resolve_call;
        timings.call_include_filter = call_timings.include_filter;
        timings.call_resolve_targets = call_timings.resolve_targets;

        let started = Instant::now();
        let imports = self.collect_imports();
        timings.imports = started.elapsed();

        let started = Instant::now();
        let annotations = self.collect_annotations();
        timings.annotations = started.elapsed();

        let started = Instant::now();
        let refs = self.parser.collect_refs();
        timings.refs = started.elapsed();

        let started = Instant::now();
        let (python_properties, python_property_callers) = self.collect_python_index_artifacts();
        timings.python_properties = started.elapsed();

        (
            FileSnapshot {
                functions,
                classes,
                fields,
                calls,
                imports,
                annotations,
                refs,
                python_properties,
                python_property_callers,
            },
            timings,
        )
    }

    pub fn build_graph_snapshot(&mut self) -> GraphFileSnapshot {
        let functions = self.collect_functions();
        self.ensure_class_snapshot();
        let classes = self.cache.classes.as_deref().unwrap_or(&[]).to_vec();
        let fields = self
            .cache
            .snapshot_fields
            .as_deref()
            .unwrap_or(&[])
            .to_vec();
        let (python_properties, python_property_callers) = self.collect_python_index_artifacts();

        GraphFileSnapshot {
            functions,
            classes,
            fields,
            calls: self.collect_calls(),
            python_properties,
            python_property_callers,
        }
    }

    pub fn find_function_callers(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
        allow_unique_method_target: bool,
    ) -> Vec<(String, Location)> {
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
        let mut seen: HashSet<(String, usize, usize)> = HashSet::new();
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
            let location = call.location.clone();
            let key = (caller.clone(), location.start_line, location.end_line);
            if seen.insert(key) {
                callers.push((caller, location));
            }
        }

        if python::with_cached_property_definitions(&self.parser, |properties| {
            properties.contains(&(function_name.to_string(), class_name.map(str::to_string)))
        }) {
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
                        caller.location.start_line,
                        class_name,
                    ) {
                        continue;
                    }
                }
                let key = (
                    caller.caller.clone(),
                    caller.location.start_line,
                    caller.location.end_line,
                );
                if seen.insert(key) {
                    callers.push((caller.caller, caller.location));
                }
            }
        }
        callers
    }

    pub fn find_function_callees(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<(String, Location)> {
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
        let mut seen: HashSet<(String, usize, usize)> = HashSet::new();
        for call in candidate_calls {
            let Some(caller) = self.enclosing_function_info_at_line(
                function_name,
                class_name,
                call.location.start_line,
            ) else {
                continue;
            };
            if call.object_name.as_deref() == Some("super()") {
                let resolved = self.resolve_super_call_targets(&caller, &call.callee);
                if !resolved.is_empty() {
                    for callee_name in resolved {
                        let key = (
                            callee_name.clone(),
                            call.location.start_line,
                            call.location.end_line,
                        );
                        if seen.insert(key) {
                            callees.push((callee_name, call.location.clone()));
                        }
                    }
                    continue;
                }
            }
            let mut callee_name = call.callee.clone();
            if let Some(ref obj) = call.object_name {
                callee_name = format!("{}.{}", obj, callee_name);
            }
            let key = (
                callee_name.clone(),
                call.location.start_line,
                call.location.end_line,
            );
            if seen.insert(key) {
                callees.push((callee_name, call.location));
            }
        }

        for property_caller in self.parser.collect_python_property_callers(None) {
            let Some(_caller) = self.enclosing_function_info_at_line(
                function_name,
                class_name,
                property_caller.location.start_line,
            ) else {
                continue;
            };
            let mut callee_name = property_caller.property_name.clone();
            if let Some(ref obj) = property_caller.object_name {
                callee_name = format!("{}.{}", obj, callee_name);
            }
            let key = (
                callee_name.clone(),
                property_caller.location.start_line,
                property_caller.location.end_line,
            );
            if seen.insert(key) {
                callees.push((callee_name, property_caller.location));
            }
        }
        callees
    }

    pub fn find_function_callees_if_present(
        &mut self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<(String, Location)> {
        let has_target = self.collect_functions().iter().any(|function| {
            function.name == function_name
                && (class_name.is_none() || function.class_name.as_deref() == class_name)
        });
        if !has_target {
            return Vec::new();
        }
        self.find_function_callees(function_name, class_name)
    }

    pub fn collect_annotations(&self) -> Vec<AnnotationInfo> {
        if !matches!(self.parser.language(), "python" | "java") {
            return Vec::new();
        }
        self.parser.collect_annotations()
    }

    fn ensure_class_snapshot(&mut self) {
        if self.cache.classes.is_some() && self.cache.snapshot_fields.is_some() {
            return;
        }
        let snapshot = self.parser.collect_class_snapshot();
        self.cache.classes = Some(snapshot.classes);
        self.cache.snapshot_fields = Some(snapshot.fields);
        self.cache.fields_by_class = snapshot.field_infos_by_class;
    }

    fn collect_python_index_artifacts(
        &self,
    ) -> (Vec<PythonPropertyInfo>, Vec<PythonPropertyCallerInfo>) {
        if self.parser.language() != "python" {
            return (Vec::new(), Vec::new());
        }
        (
            self.parser.collect_python_properties(),
            self.parser.collect_python_property_callers(None),
        )
    }

    pub fn find_refs(&mut self, name: &str) -> Vec<RefInfo> {
        self.parser.find_refs(name, true)
    }

    pub fn hydrate_refs(&mut self, candidates: &[RefInfo]) -> Vec<RefInfo> {
        self.parser.hydrate_refs(candidates)
    }
}
