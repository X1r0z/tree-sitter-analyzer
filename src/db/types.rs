use crate::models::FileSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedFileMetadata {
    pub(crate) path: String,
    pub(crate) language: String,
    pub(crate) mtime_nanos: i64,
    pub(crate) size_bytes: i64,
    pub(crate) content_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedFileSnapshot {
    pub(crate) metadata: IndexedFileMetadata,
    pub(crate) parsed: FileSnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexSyncPlan {
    pub(crate) current_files: Vec<IndexedFileMetadata>,
    pub(crate) changed_snapshots: Vec<IndexedFileSnapshot>,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedFileRow {
    pub(crate) id: i64,
    pub(crate) metadata: IndexedFileMetadata,
}
