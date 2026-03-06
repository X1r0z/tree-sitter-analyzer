use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::json;

use crate::nodes::{AnalyzerSnapshot, ClassInfo, FieldInfo, FunctionInfo, ImportInfo, Location};
use crate::utils::{is_simple_query, sort_by_file_line, QueryMatcher};

#[derive(Debug, Clone)]
pub(crate) struct IndexedFileRecord {
    pub(crate) path: String,
    pub(crate) language: String,
    pub(crate) mtime_secs: i64,
    pub(crate) size_bytes: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct FileIndexData {
    pub(crate) file: IndexedFileRecord,
    pub(crate) snapshot: AnalyzerSnapshot,
}

#[derive(Debug, Clone)]
struct IndexedFileEntry {
    id: i64,
    path: String,
    language: String,
    mtime_secs: i64,
    size_bytes: i64,
}

pub(crate) struct DbProjectAnalyzer {
    conn: Connection,
    requested_language: Option<String>,
}

impl DbProjectAnalyzer {
    pub(crate) fn from_db_file(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
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

    pub(crate) fn file_count(&self) -> usize {
        let count = if let Some(language) = self.requested_language.as_deref() {
            self.conn.query_row(
                "SELECT COUNT(*) FROM files WHERE language = ?1",
                [language],
                |row| row.get::<_, i64>(0),
            )
        } else {
            self.conn
                .query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, i64>(0))
        };
        count.map(|count| count.max(0) as usize).unwrap_or(0)
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
                mtime_secs INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL
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
            CREATE INDEX IF NOT EXISTS idx_functions_name_class ON functions(name, class_name);
            CREATE INDEX IF NOT EXISTS idx_classes_name ON classes(name);
            CREATE INDEX IF NOT EXISTS idx_class_methods_class_id ON class_methods(class_id);
            CREATE INDEX IF NOT EXISTS idx_class_super_classes_class_id ON class_super_classes(class_id);
            CREATE INDEX IF NOT EXISTS idx_class_super_classes_super_name ON class_super_classes(super_class_name);
            CREATE INDEX IF NOT EXISTS idx_fields_class_name ON fields(class_name, name);
            CREATE INDEX IF NOT EXISTS idx_fields_file_class_name ON fields(file_id, class_name);
            CREATE INDEX IF NOT EXISTS idx_calls_callee ON calls(callee);
            CREATE INDEX IF NOT EXISTS idx_calls_caller_class ON calls(caller, caller_class_name);
            CREATE INDEX IF NOT EXISTS idx_imports_module ON imports(module);
            CREATE INDEX IF NOT EXISTS idx_python_properties_name_class ON python_properties(property_name, class_name);
            CREATE INDEX IF NOT EXISTS idx_python_property_callers_name ON python_property_callers(property_name);
        ",
        )?;
        Ok(())
    }

    pub(crate) fn update_database(
        db_path: &Path,
        root_path: &str,
        language: Option<&str>,
        snapshots: &[FileIndexData],
    ) -> anyhow::Result<()> {
        let mut conn = Connection::open(db_path)?;
        Self::init_schema(&conn)?;
        let tx = conn.transaction()?;
        let existing = Self::load_metadata_map(&tx)?;
        let indexed_language = language.unwrap_or("");
        let compatible = existing.get("indexed_root_path").map(String::as_str) == Some(root_path)
            && existing
                .get("language_filter")
                .map(String::as_str)
                .unwrap_or("")
                == indexed_language;

        if !compatible {
            Self::clear_all(&tx)?;
            Self::set_metadata(&tx, "created_at", &Self::current_timestamp_string()?)?;
        }

        Self::upsert_metadata(&tx, "indexed_root_path", root_path)?;
        Self::upsert_metadata(&tx, "language_filter", indexed_language)?;
        Self::upsert_metadata(&tx, "updated_at", &Self::current_timestamp_string()?)?;

        if compatible {
            Self::sync_snapshots(&tx, snapshots)?;
        } else {
            for snapshot in snapshots {
                Self::insert_snapshot(&tx, snapshot)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn clear_all(tx: &Transaction<'_>) -> anyhow::Result<()> {
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
            DELETE FROM files;
        ",
        )?;
        Ok(())
    }

    fn set_metadata(tx: &Transaction<'_>, key: &str, value: &str) -> anyhow::Result<()> {
        tx.execute(
            "INSERT INTO metadata(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    fn upsert_metadata(tx: &Transaction<'_>, key: &str, value: &str) -> anyhow::Result<()> {
        tx.execute(
            "
            INSERT INTO metadata(key, value) VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            ",
            params![key, value],
        )?;
        Ok(())
    }

    fn load_metadata_map(tx: &Transaction<'_>) -> anyhow::Result<HashMap<String, String>> {
        let mut stmt = tx.prepare("SELECT key, value FROM metadata")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(Into::into)
    }

    fn current_timestamp_string() -> anyhow::Result<String> {
        Ok(format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
        ))
    }

    fn sync_snapshots(tx: &Transaction<'_>, snapshots: &[FileIndexData]) -> anyhow::Result<()> {
        let existing = Self::load_indexed_files(tx)?;
        let incoming: HashMap<&str, &FileIndexData> = snapshots
            .iter()
            .map(|snapshot| (snapshot.file.path.as_str(), snapshot))
            .collect();

        for (path, record) in &existing {
            if !incoming.contains_key(path.as_str()) {
                Self::delete_file_by_id(tx, record.id)?;
            }
        }

        for snapshot in snapshots {
            match existing.get(snapshot.file.path.as_str()) {
                Some(record)
                    if record.language == snapshot.file.language
                        && record.mtime_secs == snapshot.file.mtime_secs
                        && record.size_bytes == snapshot.file.size_bytes => {}
                Some(record) => {
                    Self::delete_file_by_id(tx, record.id)?;
                    Self::insert_snapshot(tx, snapshot)?;
                }
                None => Self::insert_snapshot(tx, snapshot)?,
            }
        }
        Ok(())
    }

    fn load_indexed_files(
        tx: &Transaction<'_>,
    ) -> anyhow::Result<HashMap<String, IndexedFileEntry>> {
        let mut stmt = tx.prepare(
            "
            SELECT id, path, language, mtime_secs, size_bytes
            FROM files
            ",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(IndexedFileEntry {
                id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                mtime_secs: row.get(3)?,
                size_bytes: row.get(4)?,
            })
        })?;
        let entries = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(entries
            .into_iter()
            .map(|entry| (entry.path.clone(), entry))
            .collect())
    }

    fn delete_file_by_id(tx: &Transaction<'_>, file_id: i64) -> anyhow::Result<()> {
        tx.execute("DELETE FROM files WHERE id = ?1", [file_id])?;
        Ok(())
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

    fn insert_snapshot(tx: &Transaction<'_>, snapshot: &FileIndexData) -> anyhow::Result<()> {
        tx.execute(
            "INSERT INTO files(path, language, mtime_secs, size_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![
                snapshot.file.path,
                snapshot.file.language,
                snapshot.file.mtime_secs,
                snapshot.file.size_bytes
            ],
        )?;
        let file_id = tx.last_insert_rowid();

        for function in &snapshot.snapshot.functions {
            tx.execute(
                "
                INSERT INTO functions(file_id, name, class_name, is_method, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ",
                params![
                    file_id,
                    function.name,
                    function.class_name,
                    function.is_method as i64,
                    function.location.start_line as i64,
                    function.location.end_line as i64
                ],
            )?;
        }

        for class in &snapshot.snapshot.classes {
            tx.execute(
                "
                INSERT INTO classes(file_id, name, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![
                    file_id,
                    class.name,
                    class.location.start_line as i64,
                    class.location.end_line as i64
                ],
            )?;
            let class_id = tx.last_insert_rowid();
            for method in &class.methods {
                tx.execute(
                    "INSERT INTO class_methods(class_id, method_name) VALUES (?1, ?2)",
                    params![class_id, method],
                )?;
            }
            for super_class in &class.super_classes {
                tx.execute(
                    "INSERT INTO class_super_classes(class_id, super_class_name) VALUES (?1, ?2)",
                    params![class_id, super_class],
                )?;
            }
        }

        for field in &snapshot.snapshot.fields {
            tx.execute(
                "
                INSERT INTO fields(file_id, class_name, name, field_type, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ",
                params![
                    file_id,
                    field.class_name,
                    field.name,
                    field.field_type,
                    field.location.start_line as i64,
                    field.location.end_line as i64
                ],
            )?;
        }

        for call in &snapshot.snapshot.calls {
            tx.execute(
                "
                INSERT INTO calls(file_id, callee, caller, caller_class_name, object_name, is_method_call, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ",
                params![
                    file_id,
                    call.callee,
                    call.caller,
                    call.caller_class_name,
                    call.object_name,
                    call.is_method_call as i64,
                    call.location.start_line as i64,
                    call.location.end_line as i64
                ],
            )?;
        }

        for import in &snapshot.snapshot.imports {
            tx.execute(
                "INSERT INTO imports(file_id, module, start_line) VALUES (?1, ?2, ?3)",
                params![file_id, import.module, import.location.start_line as i64],
            )?;
        }

        for property in &snapshot.snapshot.python_properties {
            tx.execute(
                "INSERT INTO python_properties(file_id, property_name, class_name) VALUES (?1, ?2, ?3)",
                params![file_id, property.name, property.class_name],
            )?;
        }

        for caller in &snapshot.snapshot.python_property_callers {
            tx.execute(
                "
                INSERT INTO python_property_callers(file_id, property_name, caller, line)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![
                    file_id,
                    caller.property_name,
                    caller.caller,
                    caller.line as i64
                ],
            )?;
        }

        Ok(())
    }

    pub(crate) fn find_functions(&self, query: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let matcher = QueryMatcher::new(query);
        let like = if !query.is_empty() && is_simple_query(query) {
            format!("%{}%", query)
        } else {
            "%".to_string()
        };
        let mut stmt = self.conn.prepare(
            "
            SELECT f.path, f.language, fn.name, fn.class_name, fn.is_method, fn.start_line, fn.end_line
            FROM functions fn
            JOIN files f ON f.id = fn.file_id
            WHERE fn.name LIKE ?1
            ORDER BY f.path, fn.start_line
            ",
        )?;
        let rows = stmt.query_map([like], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                FunctionInfo {
                    name: row.get(2)?,
                    location: Location {
                        file: row.get(0)?,
                        start_line: row.get::<_, i64>(5)? as usize,
                        end_line: row.get::<_, i64>(6)? as usize,
                    },
                    body: String::new(),
                    is_method: row.get::<_, i64>(4)? != 0,
                    class_name: row.get(3)?,
                },
            ))
        })?;

        let mut functions = Vec::new();
        for row in rows {
            let (_, language, function) = row?;
            if !self.language_matches(&language) {
                continue;
            }
            if matcher.is_match(&function.name) {
                functions.push(function);
            }
        }
        Ok(functions)
    }

    pub(crate) fn find_classes(&self, query: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let matcher = QueryMatcher::new(query);
        let like = if !query.is_empty() && is_simple_query(query) {
            format!("%{}%", query)
        } else {
            "%".to_string()
        };

        let mut stmt = self.conn.prepare(
            "
            SELECT c.id, c.file_id, f.path, f.language, c.name, c.start_line, c.end_line
            FROM classes c
            JOIN files f ON f.id = c.file_id
            WHERE c.name LIKE ?1
            ORDER BY f.path, c.start_line
            ",
        )?;
        let rows = stmt.query_map([like], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;

        let mut classes = Vec::new();
        for row in rows {
            let (class_id, file_id, file, language, name, start_line, end_line) = row?;
            if !self.language_matches(&language) {
                continue;
            }
            if !matcher.is_match(&name) {
                continue;
            }
            classes.push(ClassInfo {
                name: name.clone(),
                location: Location {
                    file,
                    start_line: start_line as usize,
                    end_line: end_line as usize,
                },
                methods: self.load_class_methods(class_id)?,
                fields: self.load_field_names(file_id, &name)?,
                super_classes: self.load_class_super_classes(class_id)?,
            });
        }
        Ok(classes)
    }

    pub(crate) fn find_fields(&self, class_name: &str) -> anyhow::Result<Vec<FieldInfo>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT f.path, f.language, fld.name, fld.field_type, fld.class_name, fld.start_line, fld.end_line
            FROM fields fld
            JOIN files f ON f.id = fld.file_id
            WHERE fld.class_name = ?1
            ORDER BY f.path, fld.start_line
            ",
        )?;
        let rows = stmt.query_map([class_name], |row| {
            Ok((
                row.get::<_, String>(1)?,
                FieldInfo {
                    name: row.get(2)?,
                    location: Location {
                        file: row.get(0)?,
                        start_line: row.get::<_, i64>(5)? as usize,
                        end_line: row.get::<_, i64>(6)? as usize,
                    },
                    field_type: row.get(3)?,
                    class_name: row.get(4)?,
                },
            ))
        })?;
        let mut fields = Vec::new();
        for row in rows {
            let (language, field) = row?;
            if self.language_matches(&language) {
                fields.push(field);
            }
        }
        Ok(fields)
    }

    pub(crate) fn find_imports(&self, query: &str) -> anyhow::Result<Vec<ImportInfo>> {
        let matcher = QueryMatcher::new(query);
        let like = if !query.is_empty() && is_simple_query(query) {
            format!("%{}%", query)
        } else {
            "%".to_string()
        };
        let mut stmt = self.conn.prepare(
            "
            SELECT f.path, f.language, i.module, i.start_line
            FROM imports i
            JOIN files f ON f.id = i.file_id
            WHERE i.module LIKE ?1
            ORDER BY f.path, i.start_line
            ",
        )?;
        let rows = stmt.query_map([like], |row| {
            Ok((
                row.get::<_, String>(1)?,
                ImportInfo {
                    module: row.get(2)?,
                    location: Location {
                        file: row.get(0)?,
                        start_line: row.get::<_, i64>(3)? as usize,
                        end_line: row.get::<_, i64>(3)? as usize,
                    },
                },
            ))
        })?;

        let mut imports = Vec::new();
        for row in rows {
            let (language, import) = row?;
            if !self.language_matches(&language) {
                continue;
            }
            if matcher.is_match(&import.module) {
                imports.push(import);
            }
        }
        Ok(imports)
    }

    pub(crate) fn find_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let (target_function, target_object) = split_function_target(function_name);
        let mut stmt = self.conn.prepare(
            "
            SELECT c.file_id, f.path, f.language, c.caller, c.caller_class_name, c.object_name, c.start_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.callee = ?1
            ORDER BY f.path, c.start_line
            ",
        )?;
        let rows = stmt.query_map([target_function], |row| {
            Ok(CallLookupRow {
                file_id: row.get(0)?,
                file: row.get(1)?,
                language: row.get(2)?,
                caller: row.get(3)?,
                caller_class_name: row.get(4)?,
                object_name: row.get(5)?,
                line: row.get::<_, i64>(6)? as usize,
            })
        })?;

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            let row = row?;
            if !self.language_matches(&row.language) {
                continue;
            }
            if let Some(target_object) = target_object {
                if row.object_name.as_deref() != Some(target_object) {
                    continue;
                }
            }
            if let Some(class_name) = class_name {
                if !self.matches_call_target_class(&row, class_name)? {
                    continue;
                }
            }
            let caller = row.caller.unwrap_or_else(|| "<module>".to_string());
            let key = (row.file.clone(), caller.clone(), row.line);
            if seen.insert(key) {
                results.push(json!({
                    "caller": caller,
                    "line": row.line,
                    "file": row.file,
                    "target_class": class_name,
                }));
            }
        }

        if self.is_python_property(function_name, class_name)? {
            let mut property_stmt = self.conn.prepare(
                "
                SELECT f.path, f.language, ppc.caller, ppc.line
                FROM python_property_callers ppc
                JOIN files f ON f.id = ppc.file_id
                WHERE ppc.property_name = ?1
                ORDER BY f.path, ppc.line
                ",
            )?;
            let property_rows = property_stmt.query_map([target_function], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? as usize,
                ))
            })?;
            for row in property_rows {
                let (file, language, caller, line) = row?;
                if !self.language_matches(&language) {
                    continue;
                }
                let key = (file.clone(), caller.clone(), line);
                if seen.insert(key) {
                    results.push(json!({
                        "caller": caller,
                        "line": line,
                        "file": file,
                        "target_class": class_name,
                    }));
                }
            }
        }

        sort_by_file_line(&mut results);
        Ok(results)
    }

    pub(crate) fn find_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let relevant_files = self.relevant_function_file_ids(function_name, class_name)?;

        let mut sql = String::from(
            "
            SELECT c.file_id, f.path, f.language, c.callee, c.object_name, c.caller_class_name, c.start_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.caller = ?1
            ",
        );
        if class_name.is_some() {
            sql.push_str(" AND c.caller_class_name = ?2");
        }
        sql.push_str(" ORDER BY f.path, c.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<CalleeLookupRow> = if let Some(class_name) = class_name {
            stmt.query_map(params![function_name, class_name], |row| {
                Ok(CalleeLookupRow {
                    file_id: row.get(0)?,
                    file: row.get(1)?,
                    language: row.get(2)?,
                    callee: row.get(3)?,
                    object_name: row.get(4)?,
                    caller_class_name: row.get(5)?,
                    line: row.get::<_, i64>(6)? as usize,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([function_name], |row| {
                Ok(CalleeLookupRow {
                    file_id: row.get(0)?,
                    file: row.get(1)?,
                    language: row.get(2)?,
                    callee: row.get(3)?,
                    object_name: row.get(4)?,
                    caller_class_name: row.get(5)?,
                    line: row.get::<_, i64>(6)? as usize,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        };

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            if !self.language_matches(&row.language) {
                continue;
            }
            if !relevant_files.is_empty() && !relevant_files.contains(&row.file_id) {
                continue;
            }
            let mut callee_name = row.callee.clone();
            if let Some(object_name) = &row.object_name {
                callee_name = format!("{}.{}", object_name, callee_name);
            }
            let key = (
                row.file.clone(),
                callee_name.clone(),
                row.caller_class_name.clone(),
            );
            if seen.insert(key) {
                results.push(json!({
                    "callee": callee_name,
                    "line": row.line,
                    "file": row.file,
                    "class_name": row.caller_class_name,
                }));
            }
        }
        sort_by_file_line(&mut results);
        Ok(results)
    }

    pub(crate) fn find_super_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let Some(target) = self.find_first_class_by_name(class_name)? else {
            return Ok(Vec::new());
        };

        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(class_name.to_string());
        queue.push_back(target);

        while let Some(current) = queue.pop_front() {
            for parent_name in &current.super_classes {
                if !visited.insert(parent_name.clone()) {
                    continue;
                }
                if let Some(parent) = self.find_first_class_by_name(parent_name)? {
                    result.push(parent.clone());
                    queue.push_back(parent);
                }
            }
        }
        Ok(result)
    }

    pub(crate) fn find_sub_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(class_name.to_string());
        queue.push_back(class_name.to_string());

        while let Some(current) = queue.pop_front() {
            for child in self.find_classes_by_super_name(&current)? {
                if visited.insert(child.name.clone()) {
                    queue.push_back(child.name.clone());
                    result.push(child);
                }
            }
        }
        Ok(result)
    }

    fn language_matches(&self, language: &str) -> bool {
        self.requested_language
            .as_deref()
            .map(|requested| requested == language)
            .unwrap_or(true)
    }

    fn find_first_class_by_name(&self, class_name: &str) -> anyhow::Result<Option<ClassInfo>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT c.id, c.file_id, f.path, f.language, c.name, c.start_line, c.end_line
            FROM classes c
            JOIN files f ON f.id = c.file_id
            WHERE c.name = ?1
            ORDER BY f.path, c.start_line
            ",
        )?;
        let rows = stmt.query_map([class_name], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;

        for row in rows {
            let (class_id, file_id, file, language, name, start_line, end_line) = row?;
            if !self.language_matches(&language) {
                continue;
            }
            return self
                .load_class_info(class_id, file_id, file, name, start_line, end_line)
                .map(Some);
        }
        Ok(None)
    }

    fn find_classes_by_super_name(&self, super_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT c.id, c.file_id, f.path, f.language, c.name, c.start_line, c.end_line
            FROM class_super_classes sc
            JOIN classes c ON c.id = sc.class_id
            JOIN files f ON f.id = c.file_id
            WHERE sc.super_class_name = ?1
            ORDER BY f.path, c.start_line
            ",
        )?;
        let rows = stmt.query_map([super_name], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;

        let mut classes = Vec::new();
        for row in rows {
            let (class_id, file_id, file, language, name, start_line, end_line) = row?;
            if !self.language_matches(&language) {
                continue;
            }
            classes
                .push(self.load_class_info(class_id, file_id, file, name, start_line, end_line)?);
        }
        Ok(classes)
    }

    fn load_class_info(
        &self,
        class_id: i64,
        file_id: i64,
        file: String,
        name: String,
        start_line: i64,
        end_line: i64,
    ) -> anyhow::Result<ClassInfo> {
        Ok(ClassInfo {
            name: name.clone(),
            location: Location {
                file,
                start_line: start_line as usize,
                end_line: end_line as usize,
            },
            methods: self.load_class_methods(class_id)?,
            fields: self.load_field_names(file_id, &name)?,
            super_classes: self.load_class_super_classes(class_id)?,
        })
    }

    fn load_class_methods(&self, class_id: i64) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT method_name FROM class_methods WHERE class_id = ?1 ORDER BY method_name",
        )?;
        let rows = stmt.query_map([class_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn load_class_super_classes(&self, class_id: i64) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT super_class_name
            FROM class_super_classes
            WHERE class_id = ?1
            ORDER BY super_class_name
            ",
        )?;
        let rows = stmt.query_map([class_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn load_field_names(&self, file_id: i64, class_name: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT name
            FROM fields
            WHERE file_id = ?1 AND class_name = ?2
            ORDER BY start_line
            ",
        )?;
        let rows = stmt.query_map(params![file_id, class_name], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn relevant_function_file_ids(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<HashSet<i64>> {
        let mut sql = String::from(
            "
            SELECT DISTINCT fn.file_id, f.language
            FROM functions fn
            JOIN files f ON f.id = fn.file_id
            WHERE fn.name = ?1
            ",
        );
        if class_name.is_some() {
            sql.push_str(" AND fn.class_name = ?2");
        }
        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<(i64, String)> = if let Some(class_name) = class_name {
            stmt.query_map(params![function_name, class_name], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([function_name], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let mut ids = HashSet::new();
        for row in rows {
            let (file_id, language) = row;
            if self.language_matches(&language) {
                ids.insert(file_id);
            }
        }
        Ok(ids)
    }

    fn is_python_property(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<bool> {
        let mut sql = String::from(
            "
            SELECT pp.property_name, f.language
            FROM python_properties pp
            JOIN files f ON f.id = pp.file_id
            WHERE pp.property_name = ?1
            ",
        );
        if class_name.is_some() {
            sql.push_str(" AND pp.class_name = ?2");
        }
        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<(String, String)> = if let Some(class_name) = class_name {
            stmt.query_map(params![function_name, class_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([function_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        for row in rows {
            let (_, language) = row;
            if self.language_matches(&language) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn matches_call_target_class(
        &self,
        row: &CallLookupRow,
        class_name: &str,
    ) -> anyhow::Result<bool> {
        if row.object_name.as_deref() == Some(class_name) {
            return Ok(true);
        }
        if row.object_name.is_none() {
            return Ok(row.caller_class_name.as_deref() == Some(class_name));
        }
        if matches!(
            row.object_name.as_deref(),
            Some("self") | Some("this") | Some("cls")
        ) {
            return Ok(row.caller_class_name.as_deref() == Some(class_name));
        }

        let (Some(object_name), Some(caller_class_name)) =
            (row.object_name.as_deref(), row.caller_class_name.as_deref())
        else {
            return Ok(false);
        };

        let Some(attr_name) = extract_instance_attr(object_name) else {
            return Ok(false);
        };

        let mut stmt = self.conn.prepare(
            "
            SELECT field_type
            FROM fields
            WHERE file_id = ?1 AND class_name = ?2 AND name = ?3
            ",
        )?;
        let rows = stmt.query_map(params![row.file_id, caller_class_name, attr_name], |row| {
            row.get::<_, Option<String>>(0)
        })?;
        for field_type in rows {
            if type_matches_class(field_type?.as_deref(), class_name) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Debug)]
struct CallLookupRow {
    file_id: i64,
    file: String,
    language: String,
    caller: Option<String>,
    caller_class_name: Option<String>,
    object_name: Option<String>,
    line: usize,
}

#[derive(Debug)]
struct CalleeLookupRow {
    file_id: i64,
    file: String,
    language: String,
    callee: String,
    object_name: Option<String>,
    caller_class_name: Option<String>,
    line: usize,
}

fn split_function_target(function_name: &str) -> (&str, Option<&str>) {
    if function_name.contains('.') {
        let parts: Vec<&str> = function_name.rsplitn(2, '.').collect();
        (parts[0], Some(parts[1]))
    } else {
        (function_name, None)
    }
}

fn extract_instance_attr(object_name: &str) -> Option<String> {
    for prefix in ["self.", "this.", "cls."] {
        if let Some(rest) = object_name.strip_prefix(prefix) {
            if !rest.is_empty() {
                return Some(rest.split('.').next().unwrap_or(rest).to_string());
            }
        }
    }
    None
}

fn type_matches_class(field_type: Option<&str>, class_name: &str) -> bool {
    let Some(field_type) = field_type else {
        return false;
    };
    field_type
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|token| !token.is_empty() && token == class_name)
}

pub(crate) fn db_path_in_current_dir() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_dir()?.join("tsa.db"))
}

pub(crate) fn file_record_from_path(
    path: &str,
    language: &str,
) -> anyhow::Result<IndexedFileRecord> {
    let metadata = std::fs::metadata(path).with_context(|| format!("Failed to stat {}", path))?;
    let modified = metadata.modified()?;
    let mtime_secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    Ok(IndexedFileRecord {
        path: path.to_string(),
        language: language.to_string(),
        mtime_secs,
        size_bytes: metadata.len() as i64,
    })
}
