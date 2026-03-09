use crate::models::{CallInfo, ClassInfo, FieldInfo, FunctionInfo, ImportInfo};
use crate::parser::{PythonPropertyCallers, PythonPropertyDefinitions};
use std::collections::HashMap;

pub(crate) struct AnalyzerCache {
    functions: Option<Vec<FunctionInfo>>,
    functions_with_bodies: Option<Vec<FunctionInfo>>,
    classes: Option<Vec<ClassInfo>>,
    calls: Option<Vec<CallInfo>>,
    calls_by_callee: Option<HashMap<String, Vec<usize>>>,
    calls_by_caller: Option<HashMap<String, Vec<usize>>>,
    python_properties: Option<PythonPropertyDefinitions>,
    python_property_callers: Option<PythonPropertyCallers>,
    imports: Option<Vec<ImportInfo>>,
    fields_by_class: HashMap<String, Vec<FieldInfo>>,
}

impl AnalyzerCache {
    pub(crate) fn new() -> Self {
        Self {
            functions: None,
            functions_with_bodies: None,
            classes: None,
            calls: None,
            calls_by_callee: None,
            calls_by_caller: None,
            python_properties: None,
            python_property_callers: None,
            imports: None,
            fields_by_class: HashMap::new(),
        }
    }

    pub(crate) fn functions(&self) -> Option<&[FunctionInfo]> {
        self.functions.as_deref()
    }

    pub(crate) fn set_functions(&mut self, functions: Vec<FunctionInfo>) {
        self.functions = Some(functions);
    }

    pub(crate) fn functions_with_bodies(&self) -> Option<&[FunctionInfo]> {
        self.functions_with_bodies.as_deref()
    }

    pub(crate) fn set_functions_with_bodies(&mut self, functions: Vec<FunctionInfo>) {
        self.functions_with_bodies = Some(functions);
    }

    pub(crate) fn classes(&self) -> Option<&[ClassInfo]> {
        self.classes.as_deref()
    }

    pub(crate) fn set_classes(&mut self, classes: Vec<ClassInfo>) {
        self.classes = Some(classes);
    }

    pub(crate) fn imports(&self) -> Option<&[ImportInfo]> {
        self.imports.as_deref()
    }

    pub(crate) fn set_imports(&mut self, imports: Vec<ImportInfo>) {
        self.imports = Some(imports);
    }

    pub(crate) fn has_calls(&self) -> bool {
        self.calls.is_some()
    }

    pub(crate) fn set_calls(&mut self, calls: Vec<CallInfo>) {
        self.calls = Some(calls);
    }

    pub(crate) fn set_calls_by_callee(&mut self, by_callee: HashMap<String, Vec<usize>>) {
        self.calls_by_callee = Some(by_callee);
    }

    pub(crate) fn set_calls_by_caller(&mut self, by_caller: HashMap<String, Vec<usize>>) {
        self.calls_by_caller = Some(by_caller);
    }

    pub(crate) fn calls(&self) -> Option<&[CallInfo]> {
        self.calls.as_deref()
    }

    pub(crate) fn calls_by_callee(&self) -> Option<&HashMap<String, Vec<usize>>> {
        self.calls_by_callee.as_ref()
    }

    pub(crate) fn calls_by_caller(&self) -> Option<&HashMap<String, Vec<usize>>> {
        self.calls_by_caller.as_ref()
    }

    pub(crate) fn set_python_properties(
        &mut self,
        properties: PythonPropertyDefinitions,
        callers: PythonPropertyCallers,
    ) {
        self.python_properties = Some(properties);
        self.python_property_callers = Some(callers);
    }

    pub(crate) fn python_properties(&self) -> Option<&PythonPropertyDefinitions> {
        self.python_properties.as_ref()
    }

    pub(crate) fn python_property_callers(&self) -> Option<&PythonPropertyCallers> {
        self.python_property_callers.as_ref()
    }

    pub(crate) fn fields(&self, class_name: &str) -> Option<&Vec<FieldInfo>> {
        self.fields_by_class.get(class_name)
    }

    pub(crate) fn insert_fields(&mut self, class_name: String, fields: Vec<FieldInfo>) {
        self.fields_by_class.insert(class_name, fields);
    }
}
