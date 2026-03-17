use std::collections::{BTreeSet, HashMap};

use rusqlite::{params_from_iter, ToSql};

use super::prefilter::RegexPrefilter;
use super::QueryContext;
use crate::models::{
    AnnotationInfo, ClassInfo, FieldInfo, FunctionInfo, ImportInfo, Location, RefInfo,
};

const SQLITE_BATCH_SIZE: usize = 256;

#[derive(Debug)]
struct FunctionRow {
    function_id: i64,
    file: String,
    name: String,
    class_name: Option<String>,
    start_line: i64,
    end_line: i64,
}

#[derive(Debug, Clone)]
struct ClassRow {
    class_id: i64,
    file_id: i64,
    file: String,
    name: String,
    kind: String,
    start_line: i64,
    end_line: i64,
}

pub(crate) struct LookupQuery<'a> {
    ctx: QueryContext<'a>,
}

impl<'a> LookupQuery<'a> {
    pub(crate) fn new(ctx: QueryContext<'a>) -> Self {
        Self { ctx }
    }

    pub(crate) fn file_count(&self) -> usize {
        let count = if let Some(language) = self.ctx.language {
            self.ctx.conn.query_row(
                "SELECT COUNT(*) FROM files WHERE language = ?1",
                [language],
                |row| row.get::<_, i64>(0),
            )
        } else {
            self.ctx
                .conn
                .query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, i64>(0))
        };
        count.map(|count| count.max(0) as usize).unwrap_or(0)
    }

    pub(crate) fn find_functions(&self, query: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let prefilter = RegexPrefilter::new(query);
        let mut sql = String::from(
            "
            SELECT fn.id, f.path, fn.name, fn.class_name, fn.start_line, fn.end_line
            FROM functions fn
            JOIN files f ON f.id = fn.file_id
            ",
        );
        let mut params: Vec<&dyn ToSql> = Vec::new();
        if let Some(fts_match_query) = prefilter.fts_match_query.as_ref() {
            sql.push_str(" JOIN functions_fts ON functions_fts.rowid = fn.id");
            sql.push_str(" WHERE functions_fts MATCH ?1 AND fn.name REGEXP ?2");
            params.push(fts_match_query);
            params.push(&query);
        } else if prefilter.match_all {
            sql.push_str(" WHERE 1 = 1");
        } else {
            sql.push_str(" WHERE fn.name REGEXP ?1");
            params.push(&query);
        }
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, fn.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(FunctionRow {
                function_id: row.get(0)?,
                file: row.get(1)?,
                name: row.get(2)?,
                class_name: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
            })
        })?;
        let function_rows = rows.collect::<Result<Vec<_>, _>>()?;
        let function_ids: Vec<_> = function_rows.iter().map(|row| row.function_id).collect();
        let params_by_function = self.load_params_by_function_id(&function_ids)?;

        Ok(function_rows
            .into_iter()
            .map(|row| FunctionInfo {
                name: row.name,
                location: Location {
                    file: row.file,
                    start_line: row.start_line as usize,
                    end_line: row.end_line as usize,
                },
                body: String::new(),
                class_name: row.class_name,
                params: params_by_function
                    .get(&row.function_id)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect())
    }

    pub(crate) fn find_classes(&self, query: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let prefilter = RegexPrefilter::new(query);
        let mut sql = String::from(
            "
            SELECT c.id, c.file_id, f.path, c.name, c.kind, c.start_line, c.end_line
            FROM classes c
            JOIN files f ON f.id = c.file_id
            ",
        );
        let mut params: Vec<&dyn ToSql> = Vec::new();
        if let Some(fts_match_query) = prefilter.fts_match_query.as_ref() {
            sql.push_str(" JOIN classes_fts ON classes_fts.rowid = c.id");
            sql.push_str(" WHERE classes_fts MATCH ?1 AND c.name REGEXP ?2");
            params.push(fts_match_query);
            params.push(&query);
        } else if prefilter.match_all {
            sql.push_str(" WHERE 1 = 1");
        } else {
            sql.push_str(" WHERE c.name REGEXP ?1");
            params.push(&query);
        }
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, c.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(ClassRow {
                class_id: row.get(0)?,
                file_id: row.get(1)?,
                file: row.get(2)?,
                name: row.get(3)?,
                kind: row.get(4)?,
                start_line: row.get(5)?,
                end_line: row.get(6)?,
            })
        })?;
        self.hydrate_class_rows(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub(crate) fn find_fields(&self, class_name: &str) -> anyhow::Result<Vec<FieldInfo>> {
        let mut sql = String::from(
            "
            SELECT f.path, fld.name, fld.field_type, fld.class_name, fld.start_line, fld.end_line
            FROM fields fld
            JOIN files f ON f.id = fld.file_id
            WHERE fld.class_name = ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&class_name];
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, fld.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(FieldInfo {
                name: row.get(1)?,
                location: Location {
                    file: row.get(0)?,
                    start_line: row.get::<_, i64>(4)? as usize,
                    end_line: row.get::<_, i64>(5)? as usize,
                },
                field_type: row.get(2)?,
                class_name: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn find_imports(&self, query: &str) -> anyhow::Result<Vec<ImportInfo>> {
        let prefilter = RegexPrefilter::new(query);
        let mut sql = String::from(
            "
            SELECT f.path, i.module, i.start_line, i.end_line
            FROM imports i
            JOIN files f ON f.id = i.file_id
            ",
        );
        let mut params: Vec<&dyn ToSql> = Vec::new();
        if let Some(fts_match_query) = prefilter.fts_match_query.as_ref() {
            sql.push_str(" JOIN imports_fts ON imports_fts.rowid = i.id");
            sql.push_str(" WHERE imports_fts MATCH ?1 AND i.module REGEXP ?2");
            params.push(fts_match_query);
            params.push(&query);
        } else if prefilter.match_all {
            sql.push_str(" WHERE 1 = 1");
        } else {
            sql.push_str(" WHERE i.module REGEXP ?1");
            params.push(&query);
        }
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, i.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(ImportInfo {
                module: row.get(1)?,
                location: Location {
                    file: row.get(0)?,
                    start_line: row.get::<_, i64>(2)? as usize,
                    end_line: row.get::<_, i64>(3)? as usize,
                },
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn find_annotations(&self, query: &str) -> anyhow::Result<Vec<AnnotationInfo>> {
        let prefilter = RegexPrefilter::new(query);
        let mut sql = String::from(
            "
            SELECT f.path, a.name, a.signature, a.start_line, a.end_line, a.target_name, a.target_type, a.target_signature
            FROM annotations a
            JOIN files f ON f.id = a.file_id
            ",
        );
        let mut params: Vec<&dyn ToSql> = Vec::new();
        if let Some(fts_match_query) = prefilter.fts_match_query.as_ref() {
            sql.push_str(" JOIN annotations_fts ON annotations_fts.rowid = a.id");
            sql.push_str(" WHERE annotations_fts MATCH ?1 AND a.name REGEXP ?2");
            params.push(fts_match_query);
            params.push(&query);
        } else if prefilter.match_all {
            sql.push_str(" WHERE 1 = 1");
        } else {
            sql.push_str(" WHERE a.name REGEXP ?1");
            params.push(&query);
        }
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, a.start_line");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(AnnotationInfo {
                name: row.get(1)?,
                signature: row.get(2)?,
                location: Location {
                    file: row.get(0)?,
                    start_line: row.get::<_, i64>(3)? as usize,
                    end_line: row.get::<_, i64>(4)? as usize,
                },
                target_name: row.get(5)?,
                target_type: row.get(6)?,
                target_signature: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn find_refs(&self, name: &str) -> anyhow::Result<Vec<RefInfo>> {
        let mut sql = String::from(
            "
            SELECT f.path, s.name, s.node_type, s.start_line, s.end_line, s.start_column, s.end_column
            FROM refs s
            JOIN files f ON f.id = s.file_id
            WHERE s.name = ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&name];
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, s.start_line, s.end_line, s.node_type");

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(RefInfo {
                name: row.get(1)?,
                node_type: row.get(2)?,
                location: Location {
                    file: row.get(0)?,
                    start_line: row.get::<_, i64>(3)? as usize,
                    end_line: row.get::<_, i64>(4)? as usize,
                },
                start_column: row.get::<_, i64>(5)? as usize,
                end_column: row.get::<_, i64>(6)? as usize,
                context: String::new(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(super) fn load_methods_by_class_id(
        &self,
        class_ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, Vec<String>>> {
        if class_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut map: HashMap<i64, Vec<String>> = HashMap::new();
        for chunk in class_ids.chunks(SQLITE_BATCH_SIZE) {
            let placeholders = repeat_placeholders(chunk.len());
            let sql = format!(
                "SELECT class_id, method_name FROM class_methods WHERE class_id IN ({placeholders}) ORDER BY class_id, method_name"
            );
            let mut stmt = self.ctx.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;

            for row in rows {
                let (class_id, method_name) = row?;
                map.entry(class_id).or_default().push(method_name);
            }
        }
        Ok(map)
    }

    pub(super) fn load_params_by_function_id(
        &self,
        function_ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, Vec<crate::models::FunctionParamInfo>>> {
        if function_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut map: HashMap<i64, Vec<crate::models::FunctionParamInfo>> = HashMap::new();
        for chunk in function_ids.chunks(SQLITE_BATCH_SIZE) {
            let placeholders = repeat_placeholders(chunk.len());
            let sql = format!(
                "SELECT function_id, name, param_type FROM function_params WHERE function_id IN ({placeholders}) ORDER BY function_id, position"
            );
            let mut stmt = self.ctx.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    crate::models::FunctionParamInfo {
                        name: row.get::<_, String>(1)?,
                        param_type: row.get(2)?,
                    },
                ))
            })?;

            for row in rows {
                let (function_id, param) = row?;
                map.entry(function_id).or_default().push(param);
            }
        }
        Ok(map)
    }

    pub(super) fn load_superclasses_by_class_id(
        &self,
        class_ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, Vec<String>>> {
        if class_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut map: HashMap<i64, Vec<String>> = HashMap::new();
        for chunk in class_ids.chunks(SQLITE_BATCH_SIZE) {
            let placeholders = repeat_placeholders(chunk.len());
            let sql = format!(
                "SELECT class_id, super_class_name FROM class_super_classes WHERE class_id IN ({placeholders}) ORDER BY class_id, super_class_name"
            );
            let mut stmt = self.ctx.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;

            for row in rows {
                let (class_id, super_class_name) = row?;
                map.entry(class_id).or_default().push(super_class_name);
            }
        }
        Ok(map)
    }

    pub(super) fn load_field_names_by_file_class(
        &self,
        file_class_pairs: &[(i64, String)],
    ) -> anyhow::Result<HashMap<(i64, String), Vec<String>>> {
        if file_class_pairs.is_empty() {
            return Ok(HashMap::new());
        }

        let mut map: HashMap<(i64, String), Vec<String>> = HashMap::new();
        for chunk in file_class_pairs.chunks(SQLITE_BATCH_SIZE) {
            let unique_pairs: Vec<(i64, String)> = chunk
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let values = std::iter::repeat_n("(?, ?)", unique_pairs.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "
                WITH lookup(file_id, class_name) AS (VALUES {values})
                SELECT fields.file_id, fields.class_name, fields.name
                FROM fields
                JOIN lookup USING (file_id, class_name)
                ORDER BY fields.file_id, fields.class_name, fields.start_line
                "
            );

            let mut bind_values: Vec<&dyn ToSql> = Vec::with_capacity(unique_pairs.len() * 2);
            for (file_id, class_name) in &unique_pairs {
                bind_values.push(file_id);
                bind_values.push(class_name);
            }

            let mut stmt = self.ctx.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(bind_values), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;

            for row in rows {
                let (file_id, class_name, field_name) = row?;
                map.entry((file_id, class_name))
                    .or_default()
                    .push(field_name);
            }
        }
        Ok(map)
    }

    pub(super) fn load_classes_by_ids(&self, class_ids: &[i64]) -> anyhow::Result<Vec<ClassInfo>> {
        if class_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut class_rows = Vec::new();
        for chunk in class_ids.chunks(SQLITE_BATCH_SIZE) {
            let placeholders = repeat_placeholders(chunk.len());
            let sql = format!(
                "
                SELECT c.id, c.file_id, f.path, c.name, c.kind, c.start_line, c.end_line
                FROM classes c
                JOIN files f ON f.id = c.file_id
                WHERE c.id IN ({placeholders})
                ORDER BY f.path, c.start_line
                "
            );
            let mut stmt = self.ctx.conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |row| {
                Ok(ClassRow {
                    class_id: row.get(0)?,
                    file_id: row.get(1)?,
                    file: row.get(2)?,
                    name: row.get(3)?,
                    kind: row.get(4)?,
                    start_line: row.get(5)?,
                    end_line: row.get(6)?,
                })
            })?;
            class_rows.extend(rows.collect::<Result<Vec<_>, _>>()?);
        }

        self.hydrate_class_rows(class_rows)
    }

    fn hydrate_class_rows(&self, class_rows: Vec<ClassRow>) -> anyhow::Result<Vec<ClassInfo>> {
        let class_ids: Vec<i64> = class_rows.iter().map(|row| row.class_id).collect();
        let field_keys: Vec<(i64, String)> = class_rows
            .iter()
            .map(|row| (row.file_id, row.name.clone()))
            .collect();
        let methods_by_class = self.load_methods_by_class_id(&class_ids)?;
        let super_classes_by_class = self.load_superclasses_by_class_id(&class_ids)?;
        let fields_by_class = self.load_field_names_by_file_class(&field_keys)?;

        Ok(class_rows
            .into_iter()
            .map(|row| ClassInfo {
                name: row.name.clone(),
                kind: row.kind,
                location: Location {
                    file: row.file,
                    start_line: row.start_line as usize,
                    end_line: row.end_line as usize,
                },
                methods: methods_by_class
                    .get(&row.class_id)
                    .cloned()
                    .unwrap_or_default(),
                fields: fields_by_class
                    .get(&(row.file_id, row.name.clone()))
                    .cloned()
                    .unwrap_or_default(),
                super_classes: super_classes_by_class
                    .get(&row.class_id)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect())
    }
}

fn repeat_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}
