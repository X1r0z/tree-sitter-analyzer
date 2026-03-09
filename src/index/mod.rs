mod builder;
pub(crate) mod query;
mod store;
mod sync;
mod types;

pub(crate) use builder::build_index;
pub(crate) use store::IndexStore;
pub(crate) use sync::{
    db_path_in_current_dir, file_record_from_path, file_record_from_path_without_hash,
    IndexSynchronizer,
};
pub(crate) use types::{FileIndexData, IndexSyncPlan, IndexedFileRecord};
