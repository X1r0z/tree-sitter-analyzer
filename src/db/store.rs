use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use regex::Regex;
use rusqlite::functions::FunctionFlags;
use rusqlite::Error;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};

use super::types::{IndexedFileEntry, IndexedFileRecord};

pub(crate) struct IndexStore {
    conn: Connection,
}

impl IndexStore {
    pub(crate) fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Self::open_connection(path)?;
        Self::ensure_schema(&conn)?;
        Ok(Self { conn })
    }

    pub(crate) fn into_connection(self) -> Connection {
        self.conn
    }

    pub(crate) fn is_compatible_with(
        &self,
        root_path: &str,
        language: Option<&str>,
    ) -> anyhow::Result<bool> {
        if self.metadata_value("indexed_root_path")?.as_deref() != Some(root_path) {
            return Ok(false);
        }
        let indexed_language = self.metadata_value("language_filter")?.unwrap_or_default();
        match language {
            Some(lang) => Ok(indexed_language.is_empty() || indexed_language == lang),
            None => Ok(indexed_language.is_empty()),
        }
    }

    pub(crate) fn open_connection(path: &Path) -> anyhow::Result<Connection> {
        let conn = Connection::open(path)?;
        Self::register_regexp_function(&conn)?;
        Ok(conn)
    }

    pub(crate) fn ensure_schema(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                language TEXT NOT NULL,
                mtime_nanos INTEGER NOT NULL DEFAULT 0,
                size_bytes INTEGER NOT NULL,
                content_hash TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS functions (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                class_name TEXT,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS function_params (
                id INTEGER PRIMARY KEY,
                function_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                param_type TEXT,
                position INTEGER NOT NULL,
                FOREIGN KEY(function_id) REFERENCES functions(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS classes (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'class',
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS class_methods (
                class_id INTEGER NOT NULL,
                method_name TEXT NOT NULL,
                FOREIGN KEY(class_id) REFERENCES classes(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS class_super_classes (
                class_id INTEGER NOT NULL,
                super_class_name TEXT NOT NULL,
                FOREIGN KEY(class_id) REFERENCES classes(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS fields (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                class_name TEXT NOT NULL,
                name TEXT NOT NULL,
                field_type TEXT,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS calls (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                callee TEXT NOT NULL,
                caller TEXT,
                caller_class_name TEXT,
                object_name TEXT,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                module TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS annotations (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                signature TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                target_name TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_signature TEXT NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS refs (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                node_type TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                start_column INTEGER NOT NULL DEFAULT 0,
                end_column INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS python_properties (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                property_name TEXT NOT NULL,
                class_name TEXT,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS python_property_callers (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                property_name TEXT NOT NULL,
                caller TEXT NOT NULL,
                caller_class_name TEXT,
                object_name TEXT,
                object_type TEXT,
                start_line INTEGER NOT NULL DEFAULT 0,
                end_line INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_language ON files(language);
            CREATE INDEX IF NOT EXISTS idx_files_language_path ON files(language, path);
            CREATE INDEX IF NOT EXISTS idx_functions_name_class ON functions(name, class_name);
            CREATE INDEX IF NOT EXISTS idx_functions_name_class_file ON functions(name, class_name, file_id);
            CREATE INDEX IF NOT EXISTS idx_functions_file_id_start_line ON functions(file_id, start_line);
            CREATE INDEX IF NOT EXISTS idx_function_params_function_id ON function_params(function_id, position);
            CREATE INDEX IF NOT EXISTS idx_function_params_function_id_name ON function_params(function_id, name);
            CREATE INDEX IF NOT EXISTS idx_classes_name ON classes(name);
            CREATE INDEX IF NOT EXISTS idx_classes_file_id_start_line ON classes(file_id, start_line);
            CREATE INDEX IF NOT EXISTS idx_class_methods_class_id ON class_methods(class_id);
            CREATE INDEX IF NOT EXISTS idx_class_super_classes_class_id ON class_super_classes(class_id);
            CREATE INDEX IF NOT EXISTS idx_class_super_classes_super_name ON class_super_classes(super_class_name);
            CREATE INDEX IF NOT EXISTS idx_class_super_classes_super_name_class_id ON class_super_classes(super_class_name, class_id);
            CREATE INDEX IF NOT EXISTS idx_fields_class_name ON fields(class_name, name);
            CREATE INDEX IF NOT EXISTS idx_fields_file_class_name ON fields(file_id, class_name);
            CREATE INDEX IF NOT EXISTS idx_fields_file_class_name_name ON fields(file_id, class_name, name);
            CREATE INDEX IF NOT EXISTS idx_fields_file_id_start_line ON fields(file_id, start_line);
            CREATE INDEX IF NOT EXISTS idx_calls_callee ON calls(callee);
            CREATE INDEX IF NOT EXISTS idx_calls_callee_file ON calls(callee, file_id);
            CREATE INDEX IF NOT EXISTS idx_calls_caller_class ON calls(caller, caller_class_name);
            CREATE INDEX IF NOT EXISTS idx_calls_caller_class_file ON calls(caller, caller_class_name, file_id);
            CREATE INDEX IF NOT EXISTS idx_calls_file_caller_class_line ON calls(file_id, caller, caller_class_name, start_line);
            CREATE INDEX IF NOT EXISTS idx_calls_file_callee_line ON calls(file_id, callee, start_line);
            CREATE INDEX IF NOT EXISTS idx_imports_module ON imports(module);
            CREATE INDEX IF NOT EXISTS idx_imports_file_id_start_line ON imports(file_id, start_line);
            CREATE INDEX IF NOT EXISTS idx_annotations_name ON annotations(name);
            CREATE INDEX IF NOT EXISTS idx_annotations_name_file ON annotations(name, file_id);
            CREATE INDEX IF NOT EXISTS idx_annotations_file_id_start_line ON annotations(file_id, start_line);
            CREATE INDEX IF NOT EXISTS idx_refs_file_id_start_end_type ON refs(file_id, start_line, end_line, node_type);
            CREATE INDEX IF NOT EXISTS idx_refs_name_file ON refs(name, file_id);
            CREATE INDEX IF NOT EXISTS idx_python_properties_name_class ON python_properties(property_name, class_name);
            CREATE INDEX IF NOT EXISTS idx_python_properties_file_name_class ON python_properties(file_id, property_name, class_name);
            CREATE INDEX IF NOT EXISTS idx_python_property_callers_name ON python_property_callers(property_name);
            CREATE INDEX IF NOT EXISTS idx_python_property_callers_name_caller_class ON python_property_callers(property_name, caller_class_name);
            CREATE INDEX IF NOT EXISTS idx_python_property_callers_caller_class_file_line ON python_property_callers(caller, caller_class_name, file_id, start_line);
            CREATE INDEX IF NOT EXISTS idx_python_property_callers_file_name_line ON python_property_callers(file_id, property_name, start_line);
            CREATE INDEX IF NOT EXISTS idx_python_property_callers_file_caller_class_line ON python_property_callers(file_id, caller, caller_class_name, start_line);
        ",
        )?;
        Self::apply_migrations(conn)?;
        Ok(())
    }

    pub(crate) fn apply_migrations(conn: &Connection) -> anyhow::Result<()> {
        let columns = Self::table_columns(conn, "files")?;
        if !columns.contains("mtime_nanos") {
            conn.execute(
                "ALTER TABLE files ADD COLUMN mtime_nanos INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !columns.contains("content_hash") {
            conn.execute(
                "ALTER TABLE files ADD COLUMN content_hash TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        let class_columns = Self::table_columns(conn, "classes")?;
        if !class_columns.contains("kind") {
            conn.execute(
                "ALTER TABLE classes ADD COLUMN kind TEXT NOT NULL DEFAULT 'class'",
                [],
            )?;
        }
        let ref_columns = Self::table_columns(conn, "refs")?;
        if !ref_columns.contains("start_column") {
            conn.execute(
                "ALTER TABLE refs ADD COLUMN start_column INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !ref_columns.contains("end_column") {
            conn.execute(
                "ALTER TABLE refs ADD COLUMN end_column INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        let import_columns = Self::table_columns(conn, "imports")?;
        if !import_columns.contains("end_line") {
            conn.execute(
                "ALTER TABLE imports ADD COLUMN end_line INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            conn.execute(
                "UPDATE imports SET end_line = start_line WHERE end_line = 0",
                [],
            )?;
        }
        let property_caller_columns = Self::table_columns(conn, "python_property_callers")?;
        if !property_caller_columns.contains("caller_class_name") {
            conn.execute(
                "ALTER TABLE python_property_callers ADD COLUMN caller_class_name TEXT",
                [],
            )?;
        }
        if !property_caller_columns.contains("object_name") {
            conn.execute(
                "ALTER TABLE python_property_callers ADD COLUMN object_name TEXT",
                [],
            )?;
        }
        if !property_caller_columns.contains("object_type") {
            conn.execute(
                "ALTER TABLE python_property_callers ADD COLUMN object_type TEXT",
                [],
            )?;
        }
        if !property_caller_columns.contains("start_line") {
            conn.execute(
                "ALTER TABLE python_property_callers ADD COLUMN start_line INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            let line_column =
                Self::table_columns(conn, "python_property_callers")?.contains("line");
            if line_column {
                conn.execute(
                    "UPDATE python_property_callers SET start_line = line WHERE start_line = 0",
                    [],
                )?;
            }
        }
        if !property_caller_columns.contains("end_line") {
            conn.execute(
                "ALTER TABLE python_property_callers ADD COLUMN end_line INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            if Self::table_columns(conn, "python_property_callers")?.contains("line") {
                conn.execute(
                    "UPDATE python_property_callers SET end_line = line WHERE end_line = 0",
                    [],
                )?;
            } else {
                conn.execute(
                    "UPDATE python_property_callers SET end_line = start_line WHERE end_line = 0",
                    [],
                )?;
            }
        }
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_python_property_callers_name_caller_class ON python_property_callers(property_name, caller_class_name)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_python_property_callers_caller_class_file_line ON python_property_callers(caller, caller_class_name, file_id, start_line)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_python_property_callers_file_name_line ON python_property_callers(file_id, property_name, start_line)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_python_property_callers_file_caller_class_line ON python_property_callers(file_id, caller, caller_class_name, start_line)",
            [],
        )?;
        Self::ensure_trigram_fts(conn, "functions_fts", "name", "functions", "id", "name")?;
        Self::ensure_trigram_fts(conn, "classes_fts", "name", "classes", "id", "name")?;
        Self::ensure_trigram_fts(conn, "imports_fts", "module", "imports", "id", "module")?;
        Self::ensure_trigram_fts(conn, "annotations_fts", "name", "annotations", "id", "name")?;
        Ok(())
    }

    pub(crate) fn clear(tx: &Transaction<'_>) -> anyhow::Result<()> {
        tx.execute_batch(
            "
            DELETE FROM metadata;
            DELETE FROM functions_fts;
            DELETE FROM classes_fts;
            DELETE FROM imports_fts;
            DELETE FROM annotations_fts;
            DELETE FROM class_methods;
            DELETE FROM class_super_classes;
            DELETE FROM python_property_callers;
            DELETE FROM python_properties;
            DELETE FROM function_params;
            DELETE FROM functions;
            DELETE FROM classes;
            DELETE FROM fields;
            DELETE FROM calls;
            DELETE FROM imports;
            DELETE FROM annotations;
            DELETE FROM refs;
            DELETE FROM files;
        ",
        )?;
        Ok(())
    }

    pub(crate) fn create_temp_current_files(
        tx: &Transaction<'_>,
        current_files: &[IndexedFileRecord],
    ) -> anyhow::Result<()> {
        tx.execute_batch(
            "
            CREATE TEMP TABLE IF NOT EXISTS temp_current_files (
                path TEXT PRIMARY KEY
            );
            DELETE FROM temp_current_files;
            ",
        )?;
        let mut stmt =
            tx.prepare_cached("INSERT OR IGNORE INTO temp_current_files(path) VALUES (?1)")?;
        for record in current_files {
            stmt.execute([record.path.as_str()])?;
        }
        Ok(())
    }

    pub(crate) fn count_deleted_files_against_temp(tx: &Transaction<'_>) -> anyhow::Result<u64> {
        let count = tx.query_row(
            "
            SELECT COUNT(*)
            FROM files
            WHERE NOT EXISTS (
                SELECT 1
                FROM temp_current_files
                WHERE temp_current_files.path = files.path
            )
            ",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count as u64)
    }

    pub(crate) fn deleted_file_ids_against_temp(tx: &Transaction<'_>) -> anyhow::Result<Vec<i64>> {
        let mut stmt = tx.prepare(
            "
            SELECT id
            FROM files
            WHERE NOT EXISTS (
                SELECT 1
                FROM temp_current_files
                WHERE temp_current_files.path = files.path
            )
            ",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn indexed_files_by_paths(
        &self,
        paths: &[String],
    ) -> anyhow::Result<HashMap<String, IndexedFileRecord>> {
        const QUERY_CHUNK_SIZE: usize = 500;

        let mut records = HashMap::new();
        for chunk in paths.chunks(QUERY_CHUNK_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let query = format!(
                "
                SELECT path, language, mtime_nanos, size_bytes, content_hash
                FROM files
                WHERE path IN ({})
                ",
                placeholders
            );
            let mut stmt = self.conn.prepare(&query)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok(IndexedFileRecord {
                    path: row.get(0)?,
                    language: row.get(1)?,
                    mtime_nanos: row.get(2)?,
                    size_bytes: row.get(3)?,
                    content_hash: row.get(4)?,
                })
            })?;
            for record in rows {
                let record = record?;
                records.insert(record.path.clone(), record);
            }
        }
        Ok(records)
    }

    pub(crate) fn indexed_file_entries_by_paths(
        tx: &Transaction<'_>,
        paths: &[String],
    ) -> anyhow::Result<HashMap<String, IndexedFileEntry>> {
        const QUERY_CHUNK_SIZE: usize = 500;

        let mut entries = HashMap::new();
        for chunk in paths.chunks(QUERY_CHUNK_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let query = format!(
                "
                SELECT id, path, language, mtime_nanos, size_bytes, content_hash
                FROM files
                WHERE path IN ({})
                ",
                placeholders
            );
            let mut stmt = tx.prepare(&query)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok(IndexedFileEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    language: row.get(2)?,
                    mtime_nanos: row.get(3)?,
                    size_bytes: row.get(4)?,
                    content_hash: row.get(5)?,
                })
            })?;
            for entry in rows {
                let entry = entry?;
                entries.insert(entry.path.clone(), entry);
            }
        }
        Ok(entries)
    }

    pub(crate) fn insert_metadata(
        tx: &Transaction<'_>,
        key: &str,
        value: &str,
    ) -> anyhow::Result<()> {
        tx.execute(
            "INSERT INTO metadata(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub(crate) fn upsert_metadata(
        tx: &Transaction<'_>,
        key: &str,
        value: &str,
    ) -> anyhow::Result<()> {
        tx.execute(
            "
            INSERT INTO metadata(key, value) VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            ",
            params![key, value],
        )?;
        Ok(())
    }

    pub(crate) fn read_metadata(tx: &Transaction<'_>) -> anyhow::Result<HashMap<String, String>> {
        let mut stmt = tx.prepare("SELECT key, value FROM metadata")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(Into::into)
    }

    pub(crate) fn current_timestamp() -> anyhow::Result<String> {
        Ok(format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
        ))
    }

    fn metadata_value(&self, key: &str) -> anyhow::Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM metadata WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(Into::into)
    }

    fn register_regexp_function(conn: &Connection) -> anyhow::Result<()> {
        type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

        conn.create_scalar_function(
            "regexp",
            2,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| {
                let regex: Arc<Regex> = ctx
                    .get_or_create_aux(0, |value| -> Result<_, BoxError> {
                        Ok(Regex::new(value.as_str()?)?)
                    })?;
                let value = ctx
                    .get_raw(1)
                    .as_str()
                    .map_err(|err| Error::UserFunctionError(err.into()))?;
                Ok(regex.is_match(value))
            },
        )?;
        Ok(())
    }

    fn ensure_trigram_fts(
        conn: &Connection,
        fts_table: &str,
        fts_column: &str,
        source_table: &str,
        source_id_column: &str,
        source_text_column: &str,
    ) -> anyhow::Result<()> {
        if Self::table_exists(conn, fts_table)? {
            return Ok(());
        }

        conn.execute(
            &format!(
                "CREATE VIRTUAL TABLE {fts_table} USING fts5({fts_column}, tokenize='trigram')"
            ),
            [],
        )?;
        conn.execute(
            &format!(
                "INSERT INTO {fts_table}(rowid, {fts_column}) SELECT {source_id_column}, {source_text_column} FROM {source_table}"
            ),
            [],
        )?;
        Ok(())
    }

    fn table_columns(conn: &Connection, table_name: &str) -> anyhow::Result<HashSet<String>> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table_name})"))?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<Result<HashSet<_>, _>>().map_err(Into::into)
    }

    fn table_exists(conn: &Connection, table_name: &str) -> anyhow::Result<bool> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type IN ('table', 'view') AND name = ?1)",
            [table_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .map_err(Into::into)
    }
}
