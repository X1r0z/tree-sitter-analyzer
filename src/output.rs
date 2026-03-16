use serde_json::{json, Value};

use crate::models::{
    AnnotationInfo, CalleeInfo, CallerInfo, ClassInfo, FieldInfo, FunctionInfo, ImportInfo, RefInfo,
};

#[derive(Clone, Copy)]
pub enum FunctionView {
    Summary,
    Definition,
}

pub fn functions(items: &[FunctionInfo], view: FunctionView) -> Value {
    Value::Array(items.iter().map(|item| function(item, view)).collect())
}

pub fn classes(items: &[ClassInfo]) -> Value {
    Value::Array(items.iter().map(class).collect())
}

pub fn fields(items: &[FieldInfo]) -> Value {
    Value::Array(items.iter().map(field).collect())
}

pub fn imports(items: &[ImportInfo]) -> Value {
    Value::Array(items.iter().map(import).collect())
}

pub fn annotations(items: &[AnnotationInfo]) -> Value {
    Value::Array(items.iter().map(annotation).collect())
}

pub fn refs(items: &[RefInfo]) -> Value {
    Value::Array(items.iter().map(ref_item).collect())
}

pub fn callers(items: &[CallerInfo]) -> Value {
    Value::Array(items.iter().map(caller).collect())
}

pub fn callees(items: &[CalleeInfo]) -> Value {
    Value::Array(items.iter().map(callee).collect())
}

fn function(item: &FunctionInfo, view: FunctionView) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), json!(item.name));
    map.insert("start_line".into(), json!(item.location.start_line));
    map.insert("end_line".into(), json!(item.location.end_line));
    map.insert("file".into(), json!(item.location.file));
    if let Some(class_name) = item.class_name.as_ref() {
        map.insert("class_name".into(), json!(class_name));
    }
    map.insert("params".into(), json!(item.params));
    if matches!(view, FunctionView::Definition) && !item.body.is_empty() {
        map.insert("body".into(), json!(item.body));
    }
    Value::Object(map)
}

fn class(item: &ClassInfo) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), json!(item.name));
    map.insert("start_line".into(), json!(item.location.start_line));
    map.insert("end_line".into(), json!(item.location.end_line));
    map.insert("methods".into(), json!(item.methods));
    map.insert("fields".into(), json!(item.fields));
    map.insert("file".into(), json!(item.location.file));
    Value::Object(map)
}

fn field(item: &FieldInfo) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), json!(item.name));
    map.insert("line".into(), json!(item.location.start_line));
    map.insert("file".into(), json!(item.location.file));
    if let Some(field_type) = item.field_type.as_ref() {
        map.insert("type".into(), json!(field_type));
    }
    Value::Object(map)
}

fn import(item: &ImportInfo) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("module".into(), json!(item.module));
    map.insert("line".into(), json!(item.location.start_line));
    map.insert("file".into(), json!(item.location.file));
    Value::Object(map)
}

fn annotation(item: &AnnotationInfo) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), json!(item.name));
    map.insert("signature".into(), json!(item.signature));
    map.insert("line".into(), json!(item.location.start_line));
    map.insert("file".into(), json!(item.location.file));
    map.insert("target_name".into(), json!(item.target_name));
    map.insert("target_type".into(), json!(item.target_type));
    map.insert("target_signature".into(), json!(item.target_signature));
    Value::Object(map)
}

fn ref_item(item: &RefInfo) -> Value {
    json!({
        "type": item.node_type,
        "location": item.location,
        "context": item.context,
    })
}

fn caller(item: &CallerInfo) -> Value {
    json!({
        "caller": item.caller,
        "line": item.line,
        "file": item.file,
    })
}

fn callee(item: &CalleeInfo) -> Value {
    json!({
        "callee": item.callee,
        "line": item.line,
        "file": item.file,
    })
}
