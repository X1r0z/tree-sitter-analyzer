use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub location: Location,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub is_method: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
}

impl FunctionInfo {
    pub fn to_json_value(&self, include_body: bool, include_file: bool) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("name".into(), json!(self.name));
        map.insert("start_line".into(), json!(self.location.start_line));
        map.insert("end_line".into(), json!(self.location.end_line));
        if include_file {
            map.insert("file".into(), json!(self.location.file));
        }
        if self.is_method {
            map.insert("is_method".into(), json!(true));
        }
        if let Some(ref cn) = self.class_name {
            map.insert("class_name".into(), json!(cn));
        }
        if include_body && !self.body.is_empty() {
            map.insert("body".into(), json!(self.body));
        }
        Value::Object(map)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassInfo {
    pub name: String,
    pub location: Location,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub super_classes: Vec<String>,
}

impl ClassInfo {
    pub fn to_json_value(&self, include_file: bool) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("name".into(), json!(self.name));
        map.insert("start_line".into(), json!(self.location.start_line));
        map.insert("end_line".into(), json!(self.location.end_line));
        map.insert("methods".into(), json!(self.methods));
        map.insert("fields".into(), json!(self.fields));
        if include_file {
            map.insert("file".into(), json!(self.location.file));
        }
        Value::Object(map)
    }
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
    #[serde(default)]
    pub is_method_call: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    pub module: String,
    pub location: Location,
}

impl ImportInfo {
    pub fn to_json_value(&self, include_file: bool) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("module".into(), json!(self.module));
        map.insert("line".into(), json!(self.location.start_line));
        if include_file {
            map.insert("file".into(), json!(self.location.file));
        }
        Value::Object(map)
    }
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

impl FieldInfo {
    pub fn to_json_value(&self, include_file: bool) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("name".into(), json!(self.name));
        map.insert("line".into(), json!(self.location.start_line));
        if include_file {
            map.insert("file".into(), json!(self.location.file));
        }
        if let Some(ref ft) = self.field_type {
            map.insert("type".into(), json!(ft));
        }
        if let Some(ref cn) = self.class_name {
            map.insert("class_name".into(), json!(cn));
        }
        Value::Object(map)
    }
}

#[derive(Debug, Clone)]
pub struct PythonPropertyInfo {
    pub name: String,
    pub class_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PythonPropertyCallerInfo {
    pub property_name: String,
    pub caller: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct AnalyzerSnapshot {
    pub functions: Vec<FunctionInfo>,
    pub classes: Vec<ClassInfo>,
    pub fields: Vec<FieldInfo>,
    pub calls: Vec<CallInfo>,
    pub imports: Vec<ImportInfo>,
    pub python_properties: Vec<PythonPropertyInfo>,
    pub python_property_callers: Vec<PythonPropertyCallerInfo>,
}
