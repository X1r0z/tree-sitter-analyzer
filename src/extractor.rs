use std::collections::HashMap;

use crate::models::{
    AnnotationInfo, CallInfo, ClassInfo, FieldInfo, FileSnapshot, FunctionInfo, ImportInfo, RefInfo,
};
use crate::parser::ParseContext;

struct ParseCache {
    functions: Option<Vec<FunctionInfo>>,
    functions_with_bodies: Option<Vec<FunctionInfo>>,
    classes: Option<Vec<ClassInfo>>,
    snapshot_fields: Option<Vec<FieldInfo>>,
    calls: Option<Vec<CallInfo>>,
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

    pub fn collect_function_definitions(
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

    pub fn collect_annotations(&self) -> Vec<AnnotationInfo> {
        if !matches!(self.parser.language(), "python" | "java") {
            return Vec::new();
        }
        self.parser.collect_annotations()
    }

    pub fn build_snapshot(&mut self) -> FileSnapshot {
        let functions = self.collect_functions();
        self.ensure_class_snapshot();
        let classes = self.cache.classes.as_deref().unwrap_or(&[]).to_vec();
        let fields = self
            .cache
            .snapshot_fields
            .as_deref()
            .unwrap_or(&[])
            .to_vec();
        let calls = self.collect_calls();
        let imports = self.collect_imports();
        let annotations = self.collect_annotations();
        let refs = self.parser.collect_refs();
        let (python_properties, python_property_callers) = if self.parser.language() == "python" {
            (
                self.parser.collect_python_properties(),
                self.parser.collect_python_property_callers(None),
            )
        } else {
            (Vec::new(), Vec::new())
        };

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
        }
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

    pub fn hydrate_refs(&mut self, candidates: &[RefInfo]) -> Vec<RefInfo> {
        self.parser.hydrate_refs(candidates)
    }
}
