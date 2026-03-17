use serde_json::{json, Value};

use crate::models::{
    AnnotationInfo, CallGraphPath, CalleeInfo, CallerInfo, ClassInfo, FieldInfo, FunctionInfo,
    ImportInfo, IndexInfo, Location, RefInfo,
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

pub fn graphs(items: &[CallGraphPath]) -> Value {
    Value::Array(items.iter().map(graph).collect())
}

pub fn index(item: &IndexInfo) -> Value {
    Value::Array(vec![
        serde_json::to_value(item).expect("index info should serialize")
    ])
}

fn function(item: &FunctionInfo, view: FunctionView) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), json!(item.name));
    map.insert("location".into(), location(&item.location));
    if let Some(class_name) = item.class_name.as_ref() {
        map.insert("class_name".into(), json!(class_name));
    }
    map.insert("params".into(), json!(item.params));
    if matches!(view, FunctionView::Definition) && !item.body.is_empty() {
        map.insert("source".into(), json!(item.body));
    }
    Value::Object(map)
}

fn class(item: &ClassInfo) -> Value {
    json!({
        "name": item.name,
        "kind": item.kind,
        "location": location(&item.location),
        "methods": item.methods,
        "fields": item.fields,
    })
}

fn field(item: &FieldInfo) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), json!(item.name));
    map.insert("location".into(), location(&item.location));
    if let Some(field_type) = item.field_type.as_ref() {
        map.insert("type".into(), json!(field_type));
    }
    Value::Object(map)
}

fn import(item: &ImportInfo) -> Value {
    json!({
        "module": item.module,
        "location": location(&item.location),
    })
}

fn annotation(item: &AnnotationInfo) -> Value {
    json!({
        "name": item.name,
        "source": item.signature,
        "location": location(&item.location),
        "target": {
            "name": item.target_name,
            "kind": item.target_type,
            "source": item.target_signature,
        }
    })
}

fn ref_item(item: &RefInfo) -> Value {
    json!({
        "name": item.name,
        "kind": item.node_type,
        "location": location(&item.location),
        "source": item.context,
    })
}

fn caller(item: &CallerInfo) -> Value {
    json!({
        "caller": item.caller,
        "location": location(&item.location),
    })
}

fn callee(item: &CalleeInfo) -> Value {
    json!({
        "callee": item.callee,
        "location": location(&item.location),
    })
}

fn graph(item: &CallGraphPath) -> Value {
    json!({
        "depth": item.depth,
        "stacktrace": item.stacktrace,
        "nodes": item.path.iter().map(|node| json!({
            "name": node.name,
            "class_name": node.class_name,
            "location": location(&node.location)
        })).collect::<Vec<_>>(),
    })
}

fn location(item: &Location) -> Value {
    json!({
        "file": item.file,
        "start_line": item.start_line,
        "end_line": item.end_line,
    })
}
