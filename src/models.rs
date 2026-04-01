use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Eq, Hash, PartialEq)]
pub struct Location {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionParamInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub location: Location,
    #[serde(default)]
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(default)]
    pub params: Vec<FunctionParamInfo>,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct FunctionKey {
    pub location: Location,
    pub name: String,
    pub class_name: Option<String>,
}

impl From<&FunctionInfo> for FunctionKey {
    fn from(function: &FunctionInfo) -> Self {
        Self {
            location: function.location.clone(),
            name: function.name.clone(),
            class_name: function.class_name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub enum GraphDirection {
    Backward,
    Forward,
}

impl GraphDirection {
    pub fn order_stacktrace<T>(self, mut frames: Vec<T>) -> Vec<T> {
        if matches!(self, Self::Forward) {
            frames.reverse();
        }
        frames
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, Hash, PartialEq)]
pub struct GraphPathNode {
    pub location: Location,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
}

impl GraphPathNode {
    pub fn display_name(&self) -> String {
        match self.class_name.as_deref() {
            Some(class_name) => format!("{}.{}", class_name, self.name),
            None => self.name.clone(),
        }
    }

    pub fn stacktrace_name(&self, file: &str, line: usize) -> String {
        let filename = Path::new(file)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(file);
        format!("{}({}:{})", self.display_name(), filename, line)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, Hash, PartialEq)]
pub struct CallGraphPath {
    pub depth: usize,
    pub stacktrace: Vec<String>,
    pub path: Vec<GraphPathNode>,
}

impl From<&FunctionInfo> for GraphPathNode {
    fn from(function: &FunctionInfo) -> Self {
        Self {
            location: function.location.clone(),
            name: function.name.clone(),
            class_name: function.class_name.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassInfo {
    pub name: String,
    pub kind: String,
    pub location: Location,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub super_classes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInfo {
    pub callee: String,
    pub location: Location,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_class_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    pub module: String,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInfo {
    pub name: String,
    pub location: Location,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationInfo {
    pub name: String,
    pub signature: String,
    pub location: Location,
    pub target_name: String,
    pub target_type: String,
    pub target_signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub location: Location,
    #[serde(skip_serializing, skip_deserializing, default)]
    pub start_column: usize,
    #[serde(skip_serializing, skip_deserializing, default)]
    pub end_column: usize,
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerInfo {
    pub caller: String,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalleeInfo {
    pub callee: String,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub database: String,
    pub candidates: usize,
    pub indexed: usize,
    pub reparsed: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct RefKey {
    pub file: String,
    pub name: String,
    pub node_type: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_column: usize,
    pub end_column: usize,
}

impl From<&RefInfo> for RefKey {
    fn from(reference: &RefInfo) -> Self {
        Self {
            file: reference.location.file.clone(),
            name: reference.name.clone(),
            node_type: reference.node_type.clone(),
            start_line: reference.location.start_line,
            end_line: reference.location.end_line,
            start_column: reference.start_column,
            end_column: reference.end_column,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PythonPropertyInfo {
    pub name: String,
    pub class_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PythonPropertyCallerInfo {
    pub location: Location,
    pub property_name: String,
    pub caller: String,
    pub caller_class_name: Option<String>,
    pub object_name: Option<String>,
    pub object_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FileSnapshot {
    pub functions: Vec<FunctionInfo>,
    pub classes: Vec<ClassInfo>,
    pub fields: Vec<FieldInfo>,
    pub calls: Vec<CallInfo>,
    pub imports: Vec<ImportInfo>,
    pub annotations: Vec<AnnotationInfo>,
    pub refs: Vec<RefInfo>,
    pub python_properties: Vec<PythonPropertyInfo>,
    pub python_property_callers: Vec<PythonPropertyCallerInfo>,
}

#[derive(Debug, Clone)]
pub struct GraphFileSnapshot {
    pub functions: Vec<FunctionInfo>,
    pub classes: Vec<ClassInfo>,
    pub fields: Vec<FieldInfo>,
    pub calls: Vec<CallInfo>,
    pub python_properties: Vec<PythonPropertyInfo>,
    pub python_property_callers: Vec<PythonPropertyCallerInfo>,
}
