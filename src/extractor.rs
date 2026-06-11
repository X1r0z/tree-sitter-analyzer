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
}

pub struct CodeExtractor {
    ctx: ParseContext,
    cache: ParseCache,
}

impl CodeExtractor {
    pub fn new(file_path: &str) -> anyhow::Result<Self> {
        let ctx = ParseContext::new(file_path)?;
        Ok(Self::from_ctx(ctx))
    }

    pub fn from_source(file_path: &str, source: Vec<u8>) -> anyhow::Result<Self> {
        let ctx = ParseContext::from_source(file_path, source)?;
        Ok(Self::from_ctx(ctx))
    }

    pub fn collect_functions(&mut self) -> Vec<FunctionInfo> {
        if self.cache.functions.is_none() {
            let mut functions = self.ctx.collect_functions(false);
            functions
                .sort_by_key(|function| (function.location.start_line, function.location.end_line));
            self.cache.functions = Some(functions);
        }
        self.cache.functions.as_deref().unwrap_or(&[]).to_vec()
    }

    pub fn collect_functions_with_bodies(&mut self) -> Vec<FunctionInfo> {
        if self.cache.functions_with_bodies.is_none() {
            let mut functions = self.ctx.collect_functions(true);
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
        if self.cache.calls.is_none() {
            self.cache.calls = Some(self.ctx.collect_calls());
        }
        self.cache.calls.as_deref().unwrap_or(&[]).to_vec()
    }

    pub fn collect_imports(&mut self) -> Vec<ImportInfo> {
        if self.cache.imports.is_none() {
            self.cache.imports = Some(self.ctx.collect_imports());
        }
        self.cache.imports.as_deref().unwrap_or(&[]).to_vec()
    }

    pub fn collect_annotations(&self) -> Vec<AnnotationInfo> {
        if !matches!(self.ctx.language(), "python" | "java") {
            return Vec::new();
        }
        self.ctx.collect_annotations()
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
        let refs = self.ctx.collect_refs();
        let (python_properties, python_property_callers) = if self.ctx.language() == "python" {
            (
                self.ctx.collect_properties(),
                self.ctx.collect_property_callers(None),
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

    pub fn hydrate_refs(&mut self, candidates: &[RefInfo]) -> Vec<RefInfo> {
        self.ctx.hydrate_refs(candidates)
    }

    fn from_ctx(ctx: ParseContext) -> Self {
        Self {
            ctx,
            cache: ParseCache {
                functions: None,
                functions_with_bodies: None,
                classes: None,
                snapshot_fields: None,
                calls: None,
                imports: None,
            },
        }
    }

    fn ensure_class_snapshot(&mut self) {
        if self.cache.classes.is_some() && self.cache.snapshot_fields.is_some() {
            return;
        }
        let snapshot = self.ctx.collect_class_snapshot();
        self.cache.classes = Some(snapshot.classes);
        self.cache.snapshot_fields = Some(snapshot.fields);
    }
}
