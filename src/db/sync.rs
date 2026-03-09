use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use indicatif::ProgressBar;
use rusqlite::Transaction;

use super::snapshot::SnapshotWriter;
use super::store::IndexStore;
use super::types::{FileIndexData, IndexSyncPlan, IndexedFileEntry, IndexedFileRecord};

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
        IndexStore::ensure_schema(&conn)?;
        let tx = conn.transaction()?;
        let metadata = IndexStore::read_metadata(&tx)?;
        progress.inc(1);
        let indexed_language = language.unwrap_or("");
        let compatible = metadata.get("indexed_root_path").map(String::as_str) == Some(root_path)
            && metadata
                .get("language_filter")
                .map(String::as_str)
                .unwrap_or("")
                == indexed_language;

        let existing_files = if compatible {
            Self::load_indexed_files_by_path(&tx)?
        } else {
            HashMap::new()
        };
        let current_paths: std::collections::HashSet<&str> = plan
            .current_files
            .iter()
            .map(|record| record.path.as_str())
            .collect();
        let deleted_files = if compatible {
            existing_files
                .keys()
                .filter(|path| !current_paths.contains(path.as_str()))
                .count()
        } else {
            0
        };
        let total_steps = 1
            + 3
            + 1
            + plan.changed_snapshots.len() as u64
            + deleted_files as u64
            + u64::from(!compatible);
        progress.set_length(total_steps);
        if !compatible {
            IndexStore::clear(&tx)?;
            progress.inc(1);
            IndexStore::insert_metadata(&tx, "created_at", &IndexStore::current_timestamp()?)?;
        }

        IndexStore::upsert_metadata(&tx, "indexed_root_path", root_path)?;
        progress.inc(1);
        IndexStore::upsert_metadata(&tx, "language_filter", indexed_language)?;
        progress.inc(1);
        IndexStore::upsert_metadata(&tx, "updated_at", &IndexStore::current_timestamp()?)?;
        progress.inc(1);

        if compatible {
            Self::sync_snapshots(
                &tx,
                &existing_files,
                &plan.current_files,
                &plan.changed_snapshots,
                progress,
            )?;
        } else {
            let mut writer = SnapshotWriter::new(&tx)?;
            for snapshot in &plan.changed_snapshots {
                writer.insert_snapshot(snapshot)?;
                progress.inc(1);
            }
        }
        tx.commit()?;
        progress.inc(1);
        Ok(())
    }

    fn sync_snapshots(
        tx: &Transaction<'_>,
        existing: &HashMap<String, IndexedFileEntry>,
        current_files: &[IndexedFileRecord],
        changed_snapshots: &[FileIndexData],
        progress: &ProgressBar,
    ) -> anyhow::Result<()> {
        let incoming: HashMap<&str, &IndexedFileRecord> = current_files
            .iter()
            .map(|record| (record.path.as_str(), record))
            .collect();

        for (path, record) in existing {
            if !incoming.contains_key(path.as_str()) {
                Self::delete_file_by_id(tx, record.id)?;
                progress.inc(1);
            }
        }

        let mut writer = SnapshotWriter::new(tx)?;
        for snapshot in changed_snapshots {
            match existing.get(snapshot.file.path.as_str()) {
                Some(record)
                    if record.language == snapshot.file.language
                        && record.mtime_nanos == snapshot.file.mtime_nanos
                        && record.size_bytes == snapshot.file.size_bytes
                        && record.content_hash == snapshot.file.content_hash => {}
                Some(record) => {
                    Self::delete_file_by_id(tx, record.id)?;
                    writer.insert_snapshot(snapshot)?;
                    progress.inc(1);
                }
                None => {
                    writer.insert_snapshot(snapshot)?;
                    progress.inc(1);
                }
            }
        }
        Ok(())
    }

    fn load_indexed_files_by_path(
        tx: &Transaction<'_>,
    ) -> anyhow::Result<HashMap<String, IndexedFileEntry>> {
        let mut stmt = tx.prepare(
            "
            SELECT id, path, language, mtime_nanos, size_bytes, content_hash
            FROM files
            ",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(IndexedFileEntry {
                id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                mtime_nanos: row.get(3)?,
                size_bytes: row.get(4)?,
                content_hash: row.get(5)?,
            })
        })?;
        let entries = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(entries
            .into_iter()
            .map(|entry| (entry.path.clone(), entry))
            .collect())
    }

    fn delete_file_by_id(tx: &Transaction<'_>, file_id: i64) -> anyhow::Result<()> {
        tx.execute(
            "DELETE FROM functions_fts WHERE rowid IN (SELECT id FROM functions WHERE file_id = ?1)",
            [file_id],
        )?;
        tx.execute(
            "DELETE FROM classes_fts WHERE rowid IN (SELECT id FROM classes WHERE file_id = ?1)",
            [file_id],
        )?;
        tx.execute(
            "DELETE FROM imports_fts WHERE rowid IN (SELECT id FROM imports WHERE file_id = ?1)",
            [file_id],
        )?;
        tx.execute(
            "DELETE FROM annotations_fts WHERE rowid IN (SELECT id FROM annotations WHERE file_id = ?1)",
            [file_id],
        )?;
        tx.execute("DELETE FROM files WHERE id = ?1", [file_id])?;
        Ok(())
    }
}

pub(crate) fn db_path_in_current_dir() -> anyhow::Result<std::path::PathBuf> {
    Ok(std::env::current_dir()?.join("tsa.db"))
}

pub(crate) fn file_record_with_hash(
    path: &str,
    language: &str,
) -> anyhow::Result<IndexedFileRecord> {
    let mut record = file_record_metadata(path, language)?;
    record.content_hash =
        blake3::hash(&std::fs::read(path).with_context(|| format!("Failed to read {}", path))?)
            .to_hex()
            .to_string();
    Ok(record)
}

pub(crate) fn file_record_metadata(
    path: &str,
    language: &str,
) -> anyhow::Result<IndexedFileRecord> {
    let metadata = std::fs::metadata(path).with_context(|| format!("Failed to stat {}", path))?;
    let modified = metadata.modified()?;
    let mtime_nanos = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64;
    Ok(IndexedFileRecord {
        path: path.to_string(),
        language: language.to_string(),
        mtime_nanos,
        size_bytes: metadata.len() as i64,
        content_hash: String::new(),
    })
}
