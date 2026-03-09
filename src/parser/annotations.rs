use super::languages::{java, python};
use super::ParseContext;
use crate::models::AnnotationInfo;

impl ParseContext {
    pub(crate) fn collect_annotations(&self) -> Vec<AnnotationInfo> {
        match self.language.as_str() {
            "java" => java::extract_java_annotations(self),
            "python" => python::extract_python_decorators(self),
            _ => Vec::new(),
        }
    }
}
