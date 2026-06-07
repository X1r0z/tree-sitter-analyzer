use std::collections::HashSet;

use rusqlite::Connection;

use super::store::IndexStore;

impl IndexStore {
    pub(crate) fn ensure_schema(conn: &Connection) -> anyhow::Result<()> {
        Self::ensure_core_schema(conn)?;
        Self::apply_migrations(conn)?;
        Self::ensure_index_schema(conn)?;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn ensure_core_schema(conn: &Connection) -> anyhow::Result<()> {
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
        ",
        )?;
        Ok(())
    }

    pub(crate) fn ensure_index_schema(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "
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
        Self::ensure_trigram_fts(conn, "functions_fts", "name", "functions", "id", "name")?;
        Self::ensure_trigram_fts(conn, "classes_fts", "name", "classes", "id", "name")?;
        Self::ensure_trigram_fts(conn, "imports_fts", "module", "imports", "id", "module")?;
        Self::ensure_trigram_fts(conn, "annotations_fts", "name", "annotations", "id", "name")?;
        Ok(())
    }

    pub(crate) fn drop_index_schema(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            "
            DROP TABLE IF EXISTS functions_fts;
            DROP TABLE IF EXISTS classes_fts;
            DROP TABLE IF EXISTS imports_fts;
            DROP TABLE IF EXISTS annotations_fts;
            DROP INDEX IF EXISTS idx_files_path;
            DROP INDEX IF EXISTS idx_files_language;
            DROP INDEX IF EXISTS idx_files_language_path;
            DROP INDEX IF EXISTS idx_functions_name_class;
            DROP INDEX IF EXISTS idx_functions_name_class_file;
            DROP INDEX IF EXISTS idx_functions_file_id_start_line;
            DROP INDEX IF EXISTS idx_function_params_function_id;
            DROP INDEX IF EXISTS idx_function_params_function_id_name;
            DROP INDEX IF EXISTS idx_classes_name;
            DROP INDEX IF EXISTS idx_classes_file_id_start_line;
            DROP INDEX IF EXISTS idx_class_methods_class_id;
            DROP INDEX IF EXISTS idx_class_super_classes_class_id;
            DROP INDEX IF EXISTS idx_class_super_classes_super_name;
            DROP INDEX IF EXISTS idx_class_super_classes_super_name_class_id;
            DROP INDEX IF EXISTS idx_fields_class_name;
            DROP INDEX IF EXISTS idx_fields_file_class_name;
            DROP INDEX IF EXISTS idx_fields_file_class_name_name;
            DROP INDEX IF EXISTS idx_fields_file_id_start_line;
            DROP INDEX IF EXISTS idx_calls_callee;
            DROP INDEX IF EXISTS idx_calls_callee_file;
            DROP INDEX IF EXISTS idx_calls_caller_class;
            DROP INDEX IF EXISTS idx_calls_caller_class_file;
            DROP INDEX IF EXISTS idx_calls_file_caller_class_line;
            DROP INDEX IF EXISTS idx_calls_file_callee_line;
            DROP INDEX IF EXISTS idx_imports_module;
            DROP INDEX IF EXISTS idx_imports_file_id_start_line;
            DROP INDEX IF EXISTS idx_annotations_name;
            DROP INDEX IF EXISTS idx_annotations_name_file;
            DROP INDEX IF EXISTS idx_annotations_file_id_start_line;
            DROP INDEX IF EXISTS idx_refs_file_id_start_end_type;
            DROP INDEX IF EXISTS idx_refs_name_file;
            DROP INDEX IF EXISTS idx_python_properties_name_class;
            DROP INDEX IF EXISTS idx_python_properties_file_name_class;
            DROP INDEX IF EXISTS idx_python_property_callers_name;
            DROP INDEX IF EXISTS idx_python_property_callers_name_caller_class;
            DROP INDEX IF EXISTS idx_python_property_callers_caller_class_file_line;
            DROP INDEX IF EXISTS idx_python_property_callers_file_name_line;
            DROP INDEX IF EXISTS idx_python_property_callers_file_caller_class_line;
            ",
        )?;
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
}
