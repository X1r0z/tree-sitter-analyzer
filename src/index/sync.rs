use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use indicatif::ProgressBar;
use rusqlite::{params, CachedStatement, Transaction};

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

struct SnapshotWriter<'tx> {
    tx: &'tx Transaction<'tx>,
    insert_file: CachedStatement<'tx>,
    insert_function: CachedStatement<'tx>,
    insert_function_fts: CachedStatement<'tx>,
    insert_function_param: CachedStatement<'tx>,
    insert_class: CachedStatement<'tx>,
    insert_class_fts: CachedStatement<'tx>,
    insert_class_method: CachedStatement<'tx>,
    insert_class_super: CachedStatement<'tx>,
    insert_field: CachedStatement<'tx>,
    insert_call: CachedStatement<'tx>,
    insert_import: CachedStatement<'tx>,
    insert_import_fts: CachedStatement<'tx>,
    insert_annotation: CachedStatement<'tx>,
    insert_annotation_fts: CachedStatement<'tx>,
    insert_symbol_ref: CachedStatement<'tx>,
    insert_python_property: CachedStatement<'tx>,
    insert_python_property_caller: CachedStatement<'tx>,
}

impl<'tx> SnapshotWriter<'tx> {
    fn new(tx: &'tx Transaction<'tx>) -> anyhow::Result<Self> {
        Ok(Self {
            tx,
            insert_file: tx.prepare_cached(
                "INSERT INTO files(path, language, mtime_nanos, size_bytes, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?,
            insert_function: tx.prepare_cached(
                "
                INSERT INTO functions(file_id, name, class_name, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
            )?,
            insert_function_fts: tx.prepare_cached(
                "INSERT INTO functions_fts(rowid, name) VALUES (?1, ?2)",
            )?,
            insert_function_param: tx.prepare_cached(
                "
                INSERT INTO function_params(function_id, name, param_type, position)
                VALUES (?1, ?2, ?3, ?4)
                ",
            )?,
            insert_class: tx.prepare_cached(
                "
                INSERT INTO classes(file_id, name, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4)
                ",
            )?,
            insert_class_fts: tx.prepare_cached(
                "INSERT INTO classes_fts(rowid, name) VALUES (?1, ?2)",
            )?,
            insert_class_method: tx.prepare_cached(
                "INSERT INTO class_methods(class_id, method_name) VALUES (?1, ?2)",
            )?,
            insert_class_super: tx.prepare_cached(
                "INSERT INTO class_super_classes(class_id, super_class_name) VALUES (?1, ?2)",
            )?,
            insert_field: tx.prepare_cached(
                "
                INSERT INTO fields(file_id, class_name, name, field_type, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ",
            )?,
            insert_call: tx.prepare_cached(
                "
                INSERT INTO calls(file_id, callee, caller, caller_class_name, object_name, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ",
            )?,
            insert_import: tx.prepare_cached(
                "INSERT INTO imports(file_id, module, start_line) VALUES (?1, ?2, ?3)",
            )?,
            insert_import_fts: tx.prepare_cached(
                "INSERT INTO imports_fts(rowid, module) VALUES (?1, ?2)",
            )?,
            insert_annotation: tx.prepare_cached(
                "
                INSERT INTO annotations(file_id, name, signature, start_line, end_line, target_name, target_type, target_signature)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ",
            )?,
            insert_annotation_fts: tx.prepare_cached(
                "INSERT INTO annotations_fts(rowid, name) VALUES (?1, ?2)",
            )?,
            insert_symbol_ref: tx.prepare_cached(
                "
                INSERT INTO symbol_refs(file_id, name, node_type, start_line, end_line, start_column, end_column)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ",
            )?,
            insert_python_property: tx.prepare_cached(
                "INSERT INTO python_properties(file_id, property_name, class_name) VALUES (?1, ?2, ?3)",
            )?,
            insert_python_property_caller: tx.prepare_cached(
                "
                INSERT INTO python_property_callers(file_id, property_name, caller, caller_class_name, object_name, line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ",
            )?,
        })
    }

    fn insert_snapshot(&mut self, snapshot: &FileIndexData) -> anyhow::Result<()> {
        self.insert_file.execute(params![
            snapshot.file.path,
            snapshot.file.language,
            snapshot.file.mtime_nanos,
            snapshot.file.size_bytes,
            snapshot.file.content_hash,
        ])?;
        let file_id = self.tx.last_insert_rowid();

        for function in &snapshot.snapshot.functions {
            self.insert_function.execute(params![
                file_id,
                function.name,
                function.class_name,
                function.location.start_line as i64,
                function.location.end_line as i64
            ])?;
            let function_id = self.tx.last_insert_rowid();
            self.insert_function_fts
                .execute(params![function_id, function.name])?;
            for (position, param) in function.params.iter().enumerate() {
                self.insert_function_param.execute(params![
                    function_id,
                    param.name,
                    param.param_type,
                    position as i64
                ])?;
            }
        }

        for class in &snapshot.snapshot.classes {
            self.insert_class.execute(params![
                file_id,
                class.name,
                class.location.start_line as i64,
                class.location.end_line as i64
            ])?;
            let class_id = self.tx.last_insert_rowid();
            self.insert_class_fts
                .execute(params![class_id, class.name])?;
            for method in &class.methods {
                self.insert_class_method
                    .execute(params![class_id, method])?;
            }
            for super_class in &class.super_classes {
                self.insert_class_super
                    .execute(params![class_id, super_class])?;
            }
        }

        for field in &snapshot.snapshot.fields {
            self.insert_field.execute(params![
                file_id,
                field.class_name,
                field.name,
                field.field_type,
                field.location.start_line as i64,
                field.location.end_line as i64
            ])?;
        }

        for call in &snapshot.snapshot.calls {
            self.insert_call.execute(params![
                file_id,
                call.callee,
                call.caller,
                call.caller_class_name,
                call.object_name,
                call.location.start_line as i64,
                call.location.end_line as i64
            ])?;
        }

        for import in &snapshot.snapshot.imports {
            self.insert_import.execute(params![
                file_id,
                import.module,
                import.location.start_line as i64
            ])?;
            let import_id = self.tx.last_insert_rowid();
            self.insert_import_fts
                .execute(params![import_id, import.module])?;
        }

        for annotation in &snapshot.snapshot.annotations {
            self.insert_annotation.execute(params![
                file_id,
                annotation.name,
                annotation.signature,
                annotation.location.start_line as i64,
                annotation.location.end_line as i64,
                annotation.target_name,
                annotation.target_type,
                annotation.target_signature
            ])?;
            let annotation_id = self.tx.last_insert_rowid();
            self.insert_annotation_fts
                .execute(params![annotation_id, annotation.name])?;
        }

        for symbol in &snapshot.snapshot.symbols {
            self.insert_symbol_ref.execute(params![
                file_id,
                symbol.name,
                symbol.node_type,
                symbol.location.start_line as i64,
                symbol.location.end_line as i64,
                symbol.start_column as i64,
                symbol.end_column as i64
            ])?;
        }

        for property in &snapshot.snapshot.python_properties {
            self.insert_python_property.execute(params![
                file_id,
                property.name,
                property.class_name
            ])?;
        }

        for caller in &snapshot.snapshot.python_property_callers {
            self.insert_python_property_caller.execute(params![
                file_id,
                caller.property_name,
                caller.caller,
                caller.caller_class_name,
                caller.object_name,
                caller.line as i64
            ])?;
        }

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
