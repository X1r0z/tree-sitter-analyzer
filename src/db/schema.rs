use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::types::IndexedFileRecord;
use super::DbProjectAnalyzer;

impl DbProjectAnalyzer {
    pub(crate) fn from_db_file(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn,
            requested_language: None,
        })
    }

    pub(crate) fn from_current_dir_if_compatible(
        root_path: &str,
        language: Option<&str>,
    ) -> anyhow::Result<Option<Self>> {
        let db_path = std::env::current_dir()?.join("tsa.db");
        if !db_path.exists() {
            return Ok(None);
        }
        let mut db = Self::from_db_file(&db_path)?;
        if db.is_compatible(root_path, language)? {
            db.requested_language = language.map(str::to_string);
            Ok(Some(db))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn init_schema(conn: &Connection) -> anyhow::Result<()> {
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
                is_method INTEGER NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS classes (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
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
                is_method_call INTEGER NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                module TEXT NOT NULL,
                start_line INTEGER NOT NULL,
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
                line INTEGER NOT NULL,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_language ON files(language);
            CREATE INDEX IF NOT EXISTS idx_files_language_path ON files(language, path);
            CREATE INDEX IF NOT EXISTS idx_functions_name_class ON functions(name, class_name);
            CREATE INDEX IF NOT EXISTS idx_functions_name_class_file ON functions(name, class_name, file_id);
            CREATE INDEX IF NOT EXISTS idx_classes_name ON classes(name);
            CREATE INDEX IF NOT EXISTS idx_class_methods_class_id ON class_methods(class_id);
            CREATE INDEX IF NOT EXISTS idx_class_super_classes_class_id ON class_super_classes(class_id);
            CREATE INDEX IF NOT EXISTS idx_class_super_classes_super_name ON class_super_classes(super_class_name);
            CREATE INDEX IF NOT EXISTS idx_class_super_classes_super_name_class_id ON class_super_classes(super_class_name, class_id);
            CREATE INDEX IF NOT EXISTS idx_fields_class_name ON fields(class_name, name);
            CREATE INDEX IF NOT EXISTS idx_fields_file_class_name ON fields(file_id, class_name);
            CREATE INDEX IF NOT EXISTS idx_fields_file_class_name_name ON fields(file_id, class_name, name);
            CREATE INDEX IF NOT EXISTS idx_calls_callee ON calls(callee);
            CREATE INDEX IF NOT EXISTS idx_calls_callee_file ON calls(callee, file_id);
            CREATE INDEX IF NOT EXISTS idx_calls_caller_class ON calls(caller, caller_class_name);
            CREATE INDEX IF NOT EXISTS idx_calls_caller_class_file ON calls(caller, caller_class_name, file_id);
            CREATE INDEX IF NOT EXISTS idx_imports_module ON imports(module);
            CREATE INDEX IF NOT EXISTS idx_annotations_name ON annotations(name);
            CREATE INDEX IF NOT EXISTS idx_annotations_name_file ON annotations(name, file_id);
            CREATE INDEX IF NOT EXISTS idx_python_properties_name_class ON python_properties(property_name, class_name);
            CREATE INDEX IF NOT EXISTS idx_python_property_callers_name ON python_property_callers(property_name);
        ",
        )?;
        Self::migrate_schema(conn)?;
        Ok(())
    }

    fn migrate_schema(conn: &Connection) -> anyhow::Result<()> {
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
        Ok(())
    }

    fn table_columns(conn: &Connection, table_name: &str) -> anyhow::Result<HashSet<String>> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table_name})"))?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<Result<HashSet<_>, _>>().map_err(Into::into)
    }

    pub(super) fn clear_all(tx: &Transaction<'_>) -> anyhow::Result<()> {
        tx.execute_batch(
            "
            DELETE FROM metadata;
            DELETE FROM class_methods;
            DELETE FROM class_super_classes;
            DELETE FROM python_property_callers;
            DELETE FROM python_properties;
            DELETE FROM functions;
            DELETE FROM classes;
            DELETE FROM fields;
            DELETE FROM calls;
            DELETE FROM imports;
            DELETE FROM annotations;
            DELETE FROM files;
        ",
        )?;
        Ok(())
    }

    pub(super) fn set_metadata(tx: &Transaction<'_>, key: &str, value: &str) -> anyhow::Result<()> {
        tx.execute(
            "INSERT INTO metadata(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub(super) fn upsert_metadata(
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

    pub(super) fn load_metadata_map(
        tx: &Transaction<'_>,
    ) -> anyhow::Result<HashMap<String, String>> {
        let mut stmt = tx.prepare("SELECT key, value FROM metadata")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(Into::into)
    }

    pub(super) fn current_timestamp_string() -> anyhow::Result<String> {
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

    pub(crate) fn is_compatible(
        &self,
        root_path: &str,
        requested_language: Option<&str>,
    ) -> anyhow::Result<bool> {
        if self.metadata_value("indexed_root_path")?.as_deref() != Some(root_path) {
            return Ok(false);
        }
        let indexed_language = self.metadata_value("language_filter")?.unwrap_or_default();
        match requested_language {
            Some(lang) => Ok(indexed_language.is_empty() || indexed_language == lang),
            None => Ok(indexed_language.is_empty()),
        }
    }

    pub(crate) fn indexed_files(&self) -> anyhow::Result<HashMap<String, IndexedFileRecord>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT path, language, mtime_nanos, size_bytes, content_hash
            FROM files
            ",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(IndexedFileRecord {
                path: row.get(0)?,
                language: row.get(1)?,
                mtime_nanos: row.get(2)?,
                size_bytes: row.get(3)?,
                content_hash: row.get(4)?,
            })
        })?;
        let records = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(records
            .into_iter()
            .map(|record| (record.path.clone(), record))
            .collect())
    }
}
