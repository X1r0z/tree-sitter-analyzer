use rusqlite::Connection;

mod prefilter;
mod query;
mod schema;
mod sync;
mod types;

pub(crate) use sync::{
    db_path_in_current_dir, file_record_from_path, file_record_without_hash_from_path,
};
pub(crate) use types::{FileIndexData, IndexSyncPlan, IndexedFileRecord};

pub(crate) struct DbProjectAnalyzer {
    conn: Connection,
    requested_language: Option<String>,
}
