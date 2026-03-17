use std::path::Path;

use anyhow::Context;
use indicatif::ProgressBar;
use rusqlite::Transaction;

use super::snapshot::SnapshotWriter;
use super::store::IndexStore;
use super::types::{FileIndexData, IndexSyncPlan, IndexedFileRecord};

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

        let deleted_files = if compatible {
            IndexStore::create_temp_current_files(&tx, &plan.current_files)?;
            IndexStore::count_deleted_files_against_temp(&tx)? as usize
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
            Self::sync_snapshots(&tx, &plan.changed_snapshots, progress)?;
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
        changed_snapshots: &[FileIndexData],
        progress: &ProgressBar,
    ) -> anyhow::Result<()> {
        for file_id in IndexStore::deleted_file_ids_against_temp(tx)? {
            Self::delete_file_by_id(tx, file_id)?;
            progress.inc(1);
        }

        let changed_paths: Vec<String> = changed_snapshots
            .iter()
            .map(|snapshot| snapshot.file.path.clone())
            .collect();
        let existing = IndexStore::indexed_file_entries_by_paths(tx, &changed_paths)?;
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
                }
                None => {
                    writer.insert_snapshot(snapshot)?;
                }
            }
            progress.inc(1);
        }
        Ok(())
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

pub(crate) fn file_record_with_hash_from_source(
    path: &str,
    language: &str,
    source: &[u8],
) -> anyhow::Result<IndexedFileRecord> {
    let mut record = file_record_metadata(path, language)?;
    record.content_hash = blake3::hash(source).to_hex().to_string();
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

#[cfg(test)]
mod tests {
    use std::fs;

    use indicatif::ProgressBar;
    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::models::{
        AnalyzerSnapshot, FunctionInfo, ImportInfo, Location, PythonPropertyCallerInfo,
        PythonPropertyInfo,
    };

    use super::*;

    #[test]
    fn sync_compatible_noop_when_file_set_and_metadata_are_unchanged() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let db_path = temp.path().join("tsa.db");
        let file_path = temp.path().join("same.py");
        write_file(&file_path, "def same():\n    return 1\n")?;

        let snapshot = make_file_index_data(&file_path, "same", "module.same")?;
        let initial_plan = IndexSyncPlan {
            current_files: vec![snapshot.file.clone()],
            changed_snapshots: vec![snapshot],
        };
        IndexSynchronizer::sync(
            &db_path,
            temp.path().to_str().unwrap_or_default(),
            None,
            &initial_plan,
            &ProgressBar::hidden(),
        )?;

        let unchanged_plan = IndexSyncPlan {
            current_files: vec![file_record_with_hash_from_source(
                file_path.to_str().unwrap_or_default(),
                "python",
                &fs::read(&file_path)?,
            )?],
            changed_snapshots: vec![],
        };
        IndexSynchronizer::sync(
            &db_path,
            temp.path().to_str().unwrap_or_default(),
            None,
            &unchanged_plan,
            &ProgressBar::hidden(),
        )?;

        let conn = IndexStore::open(&db_path)?.into_connection();
        assert_eq!(count_rows(&conn, "files")?, 1);
        assert_eq!(count_rows(&conn, "functions")?, 1);
        assert_eq!(count_rows(&conn, "imports")?, 1);
        Ok(())
    }

    #[test]
    fn sync_compatible_removes_deleted_files_and_fts_rows() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let db_path = temp.path().join("tsa.db");
        let keep_path = temp.path().join("keep.py");
        let remove_path = temp.path().join("remove.py");
        write_file(&keep_path, "def keep():\n    return 1\n")?;
        write_file(&remove_path, "def remove():\n    return 2\n")?;

        let keep_snapshot = make_file_index_data(&keep_path, "keep_fn", "keep.module")?;
        let remove_snapshot = make_file_index_data(&remove_path, "remove_fn", "remove.module")?;
        let initial_plan = IndexSyncPlan {
            current_files: vec![keep_snapshot.file.clone(), remove_snapshot.file.clone()],
            changed_snapshots: vec![keep_snapshot.clone(), remove_snapshot.clone()],
        };
        IndexSynchronizer::sync(
            &db_path,
            temp.path().to_str().unwrap_or_default(),
            None,
            &initial_plan,
            &ProgressBar::hidden(),
        )?;

        let keep_record = file_record_with_hash_from_source(
            keep_path.to_str().unwrap_or_default(),
            "python",
            &fs::read(&keep_path)?,
        )?;
        let delete_plan = IndexSyncPlan {
            current_files: vec![keep_record],
            changed_snapshots: vec![],
        };
        IndexSynchronizer::sync(
            &db_path,
            temp.path().to_str().unwrap_or_default(),
            None,
            &delete_plan,
            &ProgressBar::hidden(),
        )?;

        let conn = IndexStore::open(&db_path)?.into_connection();
        assert_eq!(count_rows(&conn, "files")?, 1);
        assert_eq!(count_rows(&conn, "functions")?, 1);
        assert_eq!(count_rows(&conn, "imports")?, 1);
        assert_eq!(count_rows(&conn, "functions_fts")?, 1);
        assert_eq!(count_rows(&conn, "imports_fts")?, 1);
        let remaining: String = conn.query_row("SELECT path FROM files", [], |row| row.get(0))?;
        assert_eq!(remaining, keep_path.to_string_lossy());
        Ok(())
    }

    #[test]
    fn sync_compatible_replaces_changed_files_without_touching_unchanged_rows() -> anyhow::Result<()>
    {
        let temp = tempdir()?;
        let db_path = temp.path().join("tsa.db");
        let file_path = temp.path().join("sample.py");
        write_file(&file_path, "def first():\n    return 1\n")?;

        let initial_snapshot = make_file_index_data(&file_path, "first", "module.one")?;
        let initial_plan = IndexSyncPlan {
            current_files: vec![initial_snapshot.file.clone()],
            changed_snapshots: vec![initial_snapshot.clone()],
        };
        IndexSynchronizer::sync(
            &db_path,
            temp.path().to_str().unwrap_or_default(),
            None,
            &initial_plan,
            &ProgressBar::hidden(),
        )?;

        write_file(&file_path, "def second():\n    return 2\n")?;
        let updated_snapshot = make_file_index_data(&file_path, "second", "module.two")?;
        let update_plan = IndexSyncPlan {
            current_files: vec![updated_snapshot.file.clone()],
            changed_snapshots: vec![updated_snapshot.clone()],
        };
        IndexSynchronizer::sync(
            &db_path,
            temp.path().to_str().unwrap_or_default(),
            None,
            &update_plan,
            &ProgressBar::hidden(),
        )?;

        let conn = IndexStore::open(&db_path)?.into_connection();
        assert_eq!(count_rows(&conn, "files")?, 1);
        assert_eq!(count_rows(&conn, "functions")?, 1);
        assert_eq!(count_rows(&conn, "imports")?, 1);
        let function_name: String =
            conn.query_row("SELECT name FROM functions", [], |row| row.get(0))?;
        let module_name: String =
            conn.query_row("SELECT module FROM imports", [], |row| row.get(0))?;
        assert_eq!(function_name, "second");
        assert_eq!(module_name, "module.two");
        Ok(())
    }

    #[test]
    fn sync_incompatible_still_clears_existing_index() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let db_path = temp.path().join("tsa.db");
        let first_root = temp.path().join("root-a");
        let second_root = temp.path().join("root-b");
        fs::create_dir_all(&first_root)?;
        fs::create_dir_all(&second_root)?;
        let first_file = first_root.join("first.py");
        let second_file = second_root.join("second.py");
        write_file(&first_file, "def first():\n    return 1\n")?;
        write_file(&second_file, "def second():\n    return 2\n")?;

        let first_snapshot = make_file_index_data(&first_file, "first", "module.first")?;
        let first_plan = IndexSyncPlan {
            current_files: vec![first_snapshot.file.clone()],
            changed_snapshots: vec![first_snapshot],
        };
        IndexSynchronizer::sync(
            &db_path,
            first_root.to_str().unwrap_or_default(),
            None,
            &first_plan,
            &ProgressBar::hidden(),
        )?;

        let second_snapshot = make_file_index_data(&second_file, "second", "module.second")?;
        let second_plan = IndexSyncPlan {
            current_files: vec![second_snapshot.file.clone()],
            changed_snapshots: vec![second_snapshot],
        };
        IndexSynchronizer::sync(
            &db_path,
            second_root.to_str().unwrap_or_default(),
            None,
            &second_plan,
            &ProgressBar::hidden(),
        )?;

        let conn = IndexStore::open(&db_path)?.into_connection();
        assert_eq!(count_rows(&conn, "files")?, 1);
        let path: String = conn.query_row("SELECT path FROM files", [], |row| row.get(0))?;
        assert_eq!(path, second_file.to_string_lossy());
        Ok(())
    }

    #[test]
    fn sync_migrates_legacy_files_table_before_sync() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let db_path = temp.path().join("tsa.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "
            CREATE TABLE metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE files (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                language TEXT NOT NULL,
                size_bytes INTEGER NOT NULL
            );
            ",
        )?;
        drop(conn);

        let file_path = temp.path().join("legacy.py");
        write_file(&file_path, "def legacy():\n    return 1\n")?;
        let snapshot = make_file_index_data(&file_path, "legacy", "module.legacy")?;
        let plan = IndexSyncPlan {
            current_files: vec![snapshot.file.clone()],
            changed_snapshots: vec![snapshot],
        };
        IndexSynchronizer::sync(
            &db_path,
            temp.path().to_str().unwrap_or_default(),
            None,
            &plan,
            &ProgressBar::hidden(),
        )?;

        let conn = IndexStore::open(&db_path)?.into_connection();
        let columns = column_names(&conn, "files")?;
        assert!(columns.iter().any(|name| name == "mtime_nanos"));
        assert!(columns.iter().any(|name| name == "content_hash"));
        assert_eq!(count_rows(&conn, "files")?, 1);
        Ok(())
    }

    fn make_file_index_data(
        path: &Path,
        function_name: &str,
        import_module: &str,
    ) -> anyhow::Result<FileIndexData> {
        let source = fs::read(path)?;
        let file = file_record_with_hash_from_source(
            path.to_str().unwrap_or_default(),
            "python",
            &source,
        )?;
        Ok(FileIndexData {
            file,
            snapshot: sample_snapshot(path, function_name, import_module),
        })
    }

    fn sample_snapshot(path: &Path, function_name: &str, import_module: &str) -> AnalyzerSnapshot {
        let file = path.to_string_lossy().to_string();
        AnalyzerSnapshot {
            functions: vec![FunctionInfo {
                name: function_name.to_string(),
                location: Location {
                    file: file.clone(),
                    start_line: 1,
                    end_line: 2,
                },
                body: String::new(),
                class_name: None,
                params: vec![],
            }],
            classes: vec![],
            fields: vec![],
            calls: vec![],
            imports: vec![ImportInfo {
                module: import_module.to_string(),
                location: Location {
                    file,
                    start_line: 1,
                    end_line: 1,
                },
            }],
            annotations: vec![],
            refs: vec![],
            python_properties: Vec::<PythonPropertyInfo>::new(),
            python_property_callers: Vec::<PythonPropertyCallerInfo>::new(),
        }
    }

    fn write_file(path: &Path, content: &str) -> anyhow::Result<()> {
        fs::write(path, content)?;
        Ok(())
    }

    fn count_rows(conn: &Connection, table: &str) -> anyhow::Result<i64> {
        let query = format!("SELECT COUNT(*) FROM {}", table);
        conn.query_row(&query, [], |row| row.get(0))
            .map_err(Into::into)
    }

    fn column_names(conn: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
        let query = format!("PRAGMA table_info({})", table);
        let mut stmt = conn.prepare(&query)?;
        let rows = stmt.query_map([], |row| row.get(1))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
