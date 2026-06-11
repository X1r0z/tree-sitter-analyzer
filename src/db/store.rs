use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;
use std::path::Path;
use std::sync::Arc;

use regex::Regex;
use rusqlite::functions::FunctionFlags;
use rusqlite::Error;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};

use super::types::{IndexedFileMetadata, IndexedFileRow};
use crate::languages::supported_language_names;
use crate::utils::collect_languages;

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

    pub(crate) fn open_connection(path: &Path) -> anyhow::Result<Connection> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA cache_size = -65536;
            PRAGMA mmap_size = 268435456;
            PRAGMA temp_store = MEMORY;
            ",
        )?;
        Self::register_regexp(&conn)?;
        Ok(conn)
    }

    pub(crate) fn is_compatible_with(
        &self,
        root_path: &str,
        language: Option<&str>,
        precomputed_languages: Option<&BTreeSet<String>>,
    ) -> anyhow::Result<bool> {
        if !self.matches_root_path(root_path)? {
            return Ok(false);
        }
        let indexed_languages = self.indexed_languages()?;
        let required_languages =
            Self::required_languages(root_path, language, precomputed_languages);
        Ok(required_languages.is_subset(&indexed_languages))
    }

    pub(crate) fn matches_root_path(&self, root_path: &str) -> anyhow::Result<bool> {
        Ok(Self::metadata_value(&self.conn, "indexed_root_path")?.as_deref() == Some(root_path))
    }

    pub(crate) fn missing_languages(
        &self,
        root_path: &str,
        language: Option<&str>,
        precomputed_languages: Option<&BTreeSet<String>>,
    ) -> anyhow::Result<Vec<String>> {
        if !self.matches_root_path(root_path)? {
            return Ok(
                Self::required_languages(root_path, language, precomputed_languages)
                    .into_iter()
                    .collect(),
            );
        }

        let indexed_languages = self.indexed_languages()?;
        let mut missing = Self::required_languages(root_path, language, precomputed_languages)
            .difference(&indexed_languages)
            .cloned()
            .collect::<Vec<_>>();
        missing.sort();
        Ok(missing)
    }

    pub(crate) fn indexed_languages(&self) -> anyhow::Result<BTreeSet<String>> {
        let indexed_languages = Self::metadata_value(&self.conn, "indexed_languages")?;
        let language_filter = Self::metadata_value(&self.conn, "language_filter")?;
        Ok(Self::languages_from_metadata(
            indexed_languages.as_deref(),
            language_filter.as_deref(),
        ))
    }

    pub(crate) fn languages_from_metadata(
        indexed_languages: Option<&str>,
        language_filter: Option<&str>,
    ) -> BTreeSet<String> {
        if let Some(value) = indexed_languages {
            return Self::deserialize_language_set(value);
        }
        match language_filter.unwrap_or_default() {
            "" => supported_language_names()
                .into_iter()
                .map(str::to_string)
                .collect(),
            filter => std::iter::once(filter.to_string()).collect(),
        }
    }

    pub(crate) fn serialize_language_set(languages: &BTreeSet<String>) -> String {
        languages.iter().cloned().collect::<Vec<_>>().join(",")
    }

    pub(crate) fn legacy_language_filter(languages: &BTreeSet<String>) -> String {
        let supported = supported_language_names()
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        if languages == &supported {
            String::new()
        } else if languages.len() == 1 {
            languages.iter().next().cloned().unwrap_or_default()
        } else {
            "__mixed__".to_string()
        }
    }

    pub(crate) fn clear(tx: &Transaction<'_>) -> anyhow::Result<()> {
        for table in [
            "functions_fts",
            "classes_fts",
            "imports_fts",
            "annotations_fts",
        ] {
            if Self::table_exists(tx, table)? {
                tx.execute(&format!("DELETE FROM {table}"), [])?;
            }
        }
        tx.execute_batch(
            "
            DELETE FROM metadata;
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

    pub(crate) fn refresh_current_files(
        tx: &Transaction<'_>,
        current_files: &[IndexedFileMetadata],
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

    pub(crate) fn count_stale_files(
        tx: &Transaction<'_>,
        scope_languages: &[String],
    ) -> anyhow::Result<u64> {
        let mut query = String::from(
            "
            SELECT COUNT(*)
            FROM files
            WHERE NOT EXISTS (
                SELECT 1
                FROM temp_current_files
                WHERE temp_current_files.path = files.path
            )
            ",
        );
        if !scope_languages.is_empty() {
            let placeholders = vec!["?"; scope_languages.len()].join(", ");
            let _ = write!(query, " AND language IN ({placeholders})");
        }

        tx.query_row(&query, params_from_iter(scope_languages.iter()), |row| {
            row.get::<_, u64>(0)
        })
        .map_err(Into::into)
    }

    pub(crate) fn stale_file_ids(
        tx: &Transaction<'_>,
        scope_languages: &[String],
    ) -> anyhow::Result<Vec<i64>> {
        let mut query = String::from(
            "
            SELECT id
            FROM files
            WHERE NOT EXISTS (
                SELECT 1
                FROM temp_current_files
                WHERE temp_current_files.path = files.path
            )
            ",
        );
        if !scope_languages.is_empty() {
            let placeholders = vec!["?"; scope_languages.len()].join(", ");
            let _ = write!(query, " AND language IN ({placeholders})");
        }
        let mut stmt = tx.prepare(&query)?;
        let rows = stmt.query_map(params_from_iter(scope_languages.iter()), |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn file_metadata_by_path(
        &self,
        paths: &[String],
    ) -> anyhow::Result<HashMap<String, IndexedFileMetadata>> {
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
                WHERE path IN ({placeholders})
                "
            );
            let mut stmt = self.conn.prepare(&query)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok(IndexedFileMetadata {
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

    pub(crate) fn file_rows_by_path(
        tx: &Transaction<'_>,
        paths: &[String],
    ) -> anyhow::Result<HashMap<String, IndexedFileRow>> {
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
                WHERE path IN ({placeholders})
                "
            );
            let mut stmt = tx.prepare(&query)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok(IndexedFileRow {
                    id: row.get(0)?,
                    metadata: IndexedFileMetadata {
                        path: row.get(1)?,
                        language: row.get(2)?,
                        mtime_nanos: row.get(3)?,
                        size_bytes: row.get(4)?,
                        content_hash: row.get(5)?,
                    },
                })
            })?;
            for entry in rows {
                let entry = entry?;
                entries.insert(entry.metadata.path.clone(), entry);
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

    pub(crate) fn metadata_value(
        conn: &Connection,
        key: &str,
    ) -> anyhow::Result<Option<String>> {
        conn.query_row("SELECT value FROM metadata WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(Into::into)
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

    pub(crate) fn table_exists(conn: &Connection, table_name: &str) -> anyhow::Result<bool> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type IN ('table', 'view') AND name = ?1)",
            [table_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .map_err(Into::into)
    }

    fn register_regexp(conn: &Connection) -> anyhow::Result<()> {
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

    fn deserialize_language_set(value: &str) -> BTreeSet<String> {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn required_languages(
        root_path: &str,
        language: Option<&str>,
        precomputed_languages: Option<&BTreeSet<String>>,
    ) -> BTreeSet<String> {
        match language {
            Some(language) => std::iter::once(language.to_string()).collect(),
            None => match precomputed_languages {
                Some(languages) => languages.clone(),
                None => collect_languages(root_path),
            },
        }
    }
}
