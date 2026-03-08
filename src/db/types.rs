use crate::models::AnalyzerSnapshot;

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
pub(crate) struct IndexedFileEntry {
    pub(crate) id: i64,
    pub(crate) path: String,
    pub(crate) language: String,
    pub(crate) mtime_nanos: i64,
    pub(crate) size_bytes: i64,
    pub(crate) content_hash: String,
}
