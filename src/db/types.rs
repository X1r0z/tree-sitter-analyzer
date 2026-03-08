use std::collections::HashMap;

use crate::nodes::{AnalyzerSnapshot, ClassInfo, FunctionInfo, FunctionKey};

#[derive(Debug, Clone)]
pub(crate) struct IndexedFileRecord {
    pub(crate) path: String,
    pub(crate) language: String,
    pub(crate) mtime_nanos: i64,
    pub(crate) size_bytes: i64,
    pub(crate) content_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FileIndexData {
    pub(crate) file: IndexedFileRecord,
    pub(crate) snapshot: AnalyzerSnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexSyncPlan {
    pub(crate) current_files: Vec<IndexedFileRecord>,
    pub(crate) changed_snapshots: Vec<FileIndexData>,
}

#[derive(Debug, Clone)]
pub(super) struct IndexedFileEntry {
    pub(super) id: i64,
    pub(super) path: String,
    pub(super) language: String,
    pub(super) mtime_nanos: i64,
    pub(super) size_bytes: i64,
    pub(super) content_hash: String,
}

pub(super) type FieldTypesByName = HashMap<String, Vec<Option<String>>>;
pub(super) type FieldTypeCache = HashMap<(i64, String), FieldTypesByName>;
pub(super) type ParamTypesByName = HashMap<String, Vec<Option<String>>>;
pub(super) type ParamTypeCache = HashMap<i64, ParamTypesByName>;

#[derive(Debug)]
pub(super) struct ClassBaseRow {
    pub(super) class_id: i64,
    pub(super) file_id: i64,
    pub(super) file: String,
    pub(super) name: String,
    pub(super) start_line: i64,
    pub(super) end_line: i64,
}

#[derive(Debug)]
pub(super) struct CallLookupRow {
    pub(super) file_id: i64,
    pub(super) file: String,
    pub(super) caller: Option<String>,
    pub(super) caller_class_name: Option<String>,
    pub(super) object_name: Option<String>,
    pub(super) line: usize,
}

#[derive(Debug)]
pub(super) struct CalleeLookupRow {
    pub(super) file_id: i64,
    pub(super) file: String,
    pub(super) callee: String,
    pub(super) object_name: Option<String>,
    pub(super) caller_class_name: Option<String>,
    pub(super) line: usize,
}

#[derive(Debug, Clone)]
pub(super) struct DbFunctionNode {
    pub(super) function_id: i64,
    pub(super) file_id: i64,
    pub(super) function: FunctionInfo,
}

impl DbFunctionNode {
    pub(super) fn key(&self) -> FunctionKey {
        FunctionKey::from_function(&self.function)
    }
}

pub(super) fn classes_by_name(classes: &[ClassInfo]) -> HashMap<String, Vec<ClassInfo>> {
    let mut map = HashMap::new();
    for class in classes {
        map.entry(class.name.clone())
            .or_insert_with(Vec::new)
            .push(class.clone());
    }
    map
}
