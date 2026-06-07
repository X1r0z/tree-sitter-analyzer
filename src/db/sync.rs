use std::path::Path;

use indicatif::ProgressBar;
use rusqlite::Transaction;

use super::snapshot::SnapshotWriter;
use super::store::IndexStore;
use super::types::{IndexSyncPlan, IndexedFileMetadata, IndexedFileSnapshot};

pub(crate) struct IndexSynchronizer;

impl IndexSynchronizer {
    pub(crate) fn sync(
        db_path: &Path,
        root_path: &str,
        language: Option<&str>,
        plan: &IndexSyncPlan,
        progress: &ProgressBar,
    ) -> anyhow::Result<()> {
        let mut conn = IndexStore::open_connection(db_path)?;
        IndexStore::ensure_core_schema(&conn)?;
        let tx = conn.transaction()?;
        let metadata = IndexStore::read_metadata(&tx)?;
        progress.inc(1);
        let same_root = metadata.get("indexed_root_path").map(String::as_str) == Some(root_path);
        let full_rebuild = !same_root;
        let scope_languages = scope_languages(language);

        let deleted_files = if same_root {
            IndexStore::refresh_temp_current_files(&tx, &plan.current_files)?;
            IndexStore::count_stale_indexed_files(&tx, &scope_languages)?
        } else {
            0
        };
        let total_steps = 1
            + 3
            + 1
            + u64::try_from(plan.changed_snapshots.len())?
            + deleted_files
            + u64::from(!same_root);
        progress.set_length(total_steps);
        if !same_root {
            IndexStore::drop_index_schema(&tx)?;
            IndexStore::clear(&tx)?;
            progress.inc(1);
            IndexStore::insert_metadata(&tx, "created_at", &IndexStore::current_timestamp()?)?;
        }

        let indexed_languages = if same_root {
            let mut languages = metadata
                .get("indexed_languages")
                .map(std::string::String::as_str)
                .map_or_else(
                    || {
                        let language_filter = metadata
                            .get("language_filter")
                            .map(String::as_str)
                            .unwrap_or_default();
                        if language_filter.is_empty() {
                            crate::languages::supported_language_names()
                                .into_iter()
                                .map(str::to_string)
                                .collect()
                        } else {
                            std::iter::once(language_filter.to_string()).collect()
                        }
                    },
                    parse_language_set,
                );
            languages.extend(scope_languages.iter().cloned());
            languages
        } else {
            scope_languages.iter().cloned().collect()
        };

        IndexStore::upsert_metadata(&tx, "indexed_root_path", root_path)?;
        progress.inc(1);
        IndexStore::upsert_metadata(
            &tx,
            "indexed_languages",
            &IndexStore::serialize_language_set(&indexed_languages),
        )?;
        IndexStore::upsert_metadata(
            &tx,
            "language_filter",
            &IndexStore::legacy_language_filter_value(&indexed_languages),
        )?;
        progress.inc(1);
        IndexStore::upsert_metadata(&tx, "updated_at", &IndexStore::current_timestamp()?)?;
        progress.inc(1);

        if same_root {
            Self::sync_snapshots(&tx, &plan.changed_snapshots, &scope_languages, progress)?;
        } else {
            let mut writer = SnapshotWriter::without_fts(&tx)?;
            for snapshot in &plan.changed_snapshots {
                writer.insert_snapshot(snapshot)?;
                progress.inc(1);
            }
        }
        if full_rebuild {
            IndexStore::ensure_index_schema(&tx)?;
        }
        tx.commit()?;
        progress.inc(1);
        Ok(())
    }

    fn sync_snapshots(
        tx: &Transaction<'_>,
        changed_snapshots: &[IndexedFileSnapshot],
        scope_languages: &[String],
        progress: &ProgressBar,
    ) -> anyhow::Result<()> {
        let stale_file_ids = IndexStore::stale_indexed_file_ids(tx, scope_languages)?;
        if !stale_file_ids.is_empty() {
            Self::delete_files_by_ids(tx, &stale_file_ids)?;
            for _ in &stale_file_ids {
                progress.inc(1);
            }
        }

        let changed_paths: Vec<String> = changed_snapshots
            .iter()
            .map(|snapshot| snapshot.metadata.path.clone())
            .collect();
        let existing = IndexStore::indexed_file_rows_by_path(tx, &changed_paths)?;
        let mut replaced_file_ids = Vec::new();
        for snapshot in changed_snapshots {
            if let Some(record) = existing.get(snapshot.metadata.path.as_str()) {
                let unchanged = record.language == snapshot.metadata.language
                    && record.mtime_nanos == snapshot.metadata.mtime_nanos
                    && record.size_bytes == snapshot.metadata.size_bytes
                    && record.content_hash == snapshot.metadata.content_hash;
                if !unchanged {
                    replaced_file_ids.push(record.id);
                }
            }
        }
        if !replaced_file_ids.is_empty() {
            Self::delete_files_by_ids(tx, &replaced_file_ids)?;
        }

        let mut writer = SnapshotWriter::new(tx)?;
        for snapshot in changed_snapshots {
            match existing.get(snapshot.metadata.path.as_str()) {
                Some(record)
                    if record.language == snapshot.metadata.language
                        && record.mtime_nanos == snapshot.metadata.mtime_nanos
                        && record.size_bytes == snapshot.metadata.size_bytes
                        && record.content_hash == snapshot.metadata.content_hash => {}
                Some(_) | None => {
                    writer.insert_snapshot(snapshot)?;
                }
            }
            progress.inc(1);
        }
        Ok(())
    }

    fn delete_files_by_ids(tx: &Transaction<'_>, file_ids: &[i64]) -> anyhow::Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }

        let placeholders = vec!["?"; file_ids.len()].join(", ");
        for table in [
            "functions_fts",
            "classes_fts",
            "imports_fts",
            "annotations_fts",
        ] {
            if IndexStore::table_exists(tx, table)? {
                let source_table = table.trim_end_matches("_fts");
                tx.execute(
                    &format!(
                        "DELETE FROM {table} WHERE rowid IN (SELECT id FROM {source_table} WHERE file_id IN ({placeholders}))"
                    ),
                    rusqlite::params_from_iter(file_ids.iter()),
                )?;
            }
        }
        tx.execute(
            &format!("DELETE FROM files WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(file_ids.iter()),
        )?;
        Ok(())
    }
}

fn scope_languages(language: Option<&str>) -> Vec<String> {
    match language {
        Some(language) => vec![language.to_string()],
        None => crate::languages::supported_language_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
    }
}

fn parse_language_set(value: &str) -> std::collections::BTreeSet<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

pub(crate) fn db_path_in_current_dir() -> anyhow::Result<std::path::PathBuf> {
    Ok(std::env::current_dir()?.join("tsa.db"))
}

pub(crate) fn file_record_from_metadata(
    path: &str,
    language: &str,
    metadata: &std::fs::Metadata,
) -> anyhow::Result<IndexedFileMetadata> {
    let modified = metadata.modified()?;
    let mtime_nanos = i64::try_from(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    )?;
    Ok(IndexedFileMetadata {
        path: path.to_string(),
        language: language.to_string(),
        mtime_nanos,
        size_bytes: i64::try_from(metadata.len())?,
        content_hash: String::new(),
    })
}
