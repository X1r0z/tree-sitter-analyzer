use std::collections::{HashMap, HashSet, VecDeque};

use rusqlite::{params, params_from_iter, OptionalExtension, ToSql};
use serde_json::json;

use crate::nodes::{
    AnnotationInfo, ClassInfo, FieldInfo, FunctionInfo, ImportInfo, Location, SymbolRefInfo,
};
use crate::utils::{is_simple_query, sort_by_file_line, QueryMatcher};

use super::helpers::{
    extract_instance_attr, repeat_placeholders, split_function_target, type_matches_class,
};
use super::types::{
    classes_by_name, CallLookupRow, CalleeLookupRow, ClassBaseRow, FieldTypeCache, FieldTypesByName,
};
use super::DbProjectAnalyzer;

impl DbProjectAnalyzer {
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

    pub(crate) fn find_functions(&self, query: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        let matcher = QueryMatcher::new(query);
        let like = if !query.is_empty() && is_simple_query(query) {
            format!("%{}%", query)
        } else {
            "%".to_string()
        };
        let mut sql = String::from(
            "
            SELECT f.path, fn.name, fn.class_name, fn.is_method, fn.start_line, fn.end_line
            FROM functions fn
            JOIN files f ON f.id = fn.file_id
            WHERE fn.name LIKE ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&like];
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, fn.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(FunctionInfo {
                name: row.get(1)?,
                location: Location {
                    file: row.get(0)?,
                    start_line: row.get::<_, i64>(4)? as usize,
                    end_line: row.get::<_, i64>(5)? as usize,
                },
                body: String::new(),
                is_method: row.get::<_, i64>(3)? != 0,
                class_name: row.get(2)?,
                params: Vec::new(),
            })
        })?;

        rows.filter_map(|row| match row {
            Ok(function) if matcher.is_match(&function.name) => Some(Ok(function)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    pub(crate) fn find_classes(&self, query: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let matcher = QueryMatcher::new(query);
        let like = if !query.is_empty() && is_simple_query(query) {
            format!("%{}%", query)
        } else {
            "%".to_string()
        };

        let mut sql = String::from(
            "
            SELECT c.id, c.file_id, f.path, c.name, c.start_line, c.end_line
            FROM classes c
            JOIN files f ON f.id = c.file_id
            WHERE c.name LIKE ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&like];
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, c.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(ClassBaseRow {
                class_id: row.get(0)?,
                file_id: row.get(1)?,
                file: row.get(2)?,
                name: row.get(3)?,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
            })
        })?;
        let class_rows = rows
            .filter_map(|row| match row {
                Ok(class_row) if matcher.is_match(&class_row.name) => Some(Ok(class_row)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let class_ids: Vec<i64> = class_rows.iter().map(|row| row.class_id).collect();
        let field_keys: Vec<(i64, String)> = class_rows
            .iter()
            .map(|row| (row.file_id, row.name.clone()))
            .collect();
        let methods_by_class = self.load_class_methods_map(&class_ids)?;
        let super_classes_by_class = self.load_class_super_classes_map(&class_ids)?;
        let fields_by_class = self.load_field_names_map(&field_keys)?;

        Ok(class_rows
            .into_iter()
            .map(|row| ClassInfo {
                name: row.name.clone(),
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
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, fld.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
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
        let matcher = QueryMatcher::new(query);
        let like = if !query.is_empty() && is_simple_query(query) {
            format!("%{}%", query)
        } else {
            "%".to_string()
        };
        let mut sql = String::from(
            "
            SELECT f.path, i.module, i.start_line
            FROM imports i
            JOIN files f ON f.id = i.file_id
            WHERE i.module LIKE ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&like];
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, i.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(ImportInfo {
                module: row.get(1)?,
                location: Location {
                    file: row.get(0)?,
                    start_line: row.get::<_, i64>(2)? as usize,
                    end_line: row.get::<_, i64>(2)? as usize,
                },
            })
        })?;

        rows.filter_map(|row| match row {
            Ok(import) if matcher.is_match(&import.module) => Some(Ok(import)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    pub(crate) fn find_annotations(&self, query: &str) -> anyhow::Result<Vec<AnnotationInfo>> {
        let matcher = QueryMatcher::new(query);
        let like = if !query.is_empty() && is_simple_query(query) {
            format!("%{}%", query)
        } else {
            "%".to_string()
        };
        let mut sql = String::from(
            "
            SELECT f.path, a.name, a.signature, a.start_line, a.end_line, a.target_name, a.target_type, a.target_signature
            FROM annotations a
            JOIN files f ON f.id = a.file_id
            WHERE a.name LIKE ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&like];
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, a.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
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

        rows.filter_map(|row| match row {
            Ok(annotation) if matcher.is_match(&annotation.name) => Some(Ok(annotation)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    pub(crate) fn find_symbols(&self, name: &str) -> anyhow::Result<Vec<SymbolRefInfo>> {
        let mut sql = String::from(
            "
            SELECT f.path, s.name, s.node_type, s.start_line, s.end_line, s.start_column, s.end_column
            FROM symbol_refs s
            JOIN files f ON f.id = s.file_id
            WHERE s.name = ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&name];
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, s.start_line, s.end_line, s.node_type");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(SymbolRefInfo {
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

    pub(crate) fn find_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let (target_function, target_object) = split_function_target(function_name);
        let mut sql = String::from(
            "
            SELECT c.file_id, f.path, c.caller, c.caller_class_name, c.object_name, c.start_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.callee = ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&target_function];
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, c.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(CallLookupRow {
                file_id: row.get(0)?,
                file: row.get(1)?,
                caller: row.get(2)?,
                caller_class_name: row.get(3)?,
                object_name: row.get(4)?,
                line: row.get::<_, i64>(5)? as usize,
            })
        })?;

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        let mut field_type_cache = HashMap::new();
        for row in rows {
            let row = row?;
            if let Some(target_object) = target_object {
                if row.object_name.as_deref() != Some(target_object) {
                    continue;
                }
            }
            if let Some(class_name) = class_name {
                if !self.matches_call_target_class(&row, class_name, &mut field_type_cache)? {
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
            let mut property_sql = String::from(
                "
                SELECT f.path, ppc.caller, ppc.line
                FROM python_property_callers ppc
                JOIN files f ON f.id = ppc.file_id
                WHERE ppc.property_name = ?1
                ",
            );
            let mut property_params: Vec<&dyn ToSql> = vec![&target_function];
            if let Some(language) = self.requested_language.as_ref() {
                property_sql.push_str(" AND f.language = ?2");
                property_params.push(language);
            }
            property_sql.push_str(" ORDER BY f.path, ppc.line");
            let mut property_stmt = self.conn.prepare(&property_sql)?;
            let property_rows =
                property_stmt.query_map(params_from_iter(property_params), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? as usize,
                    ))
                })?;
            for row in property_rows {
                let (file, caller, line) = row?;
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
            SELECT c.file_id, f.path, c.callee, c.object_name, c.caller_class_name, c.start_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.caller = ?1
            ",
        );
        let language = self.requested_language.as_deref();
        if class_name.is_some() {
            sql.push_str(" AND c.caller_class_name = ?2");
        }
        if language.is_some() {
            sql.push_str(if class_name.is_some() {
                " AND f.language = ?3"
            } else {
                " AND f.language = ?2"
            });
        }
        sql.push_str(" ORDER BY f.path, c.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<CalleeLookupRow> = match (class_name, language) {
            (Some(class_name), Some(language)) => stmt
                .query_map(params![function_name, class_name, language], |row| {
                    Ok(CalleeLookupRow {
                        file_id: row.get(0)?,
                        file: row.get(1)?,
                        callee: row.get(2)?,
                        object_name: row.get(3)?,
                        caller_class_name: row.get(4)?,
                        line: row.get::<_, i64>(5)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(class_name), None) => stmt
                .query_map(params![function_name, class_name], |row| {
                    Ok(CalleeLookupRow {
                        file_id: row.get(0)?,
                        file: row.get(1)?,
                        callee: row.get(2)?,
                        object_name: row.get(3)?,
                        caller_class_name: row.get(4)?,
                        line: row.get::<_, i64>(5)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(language)) => stmt
                .query_map(params![function_name, language], |row| {
                    Ok(CalleeLookupRow {
                        file_id: row.get(0)?,
                        file: row.get(1)?,
                        callee: row.get(2)?,
                        object_name: row.get(3)?,
                        caller_class_name: row.get(4)?,
                        line: row.get::<_, i64>(5)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([function_name], |row| {
                    Ok(CalleeLookupRow {
                        file_id: row.get(0)?,
                        file: row.get(1)?,
                        callee: row.get(2)?,
                        object_name: row.get(3)?,
                        caller_class_name: row.get(4)?,
                        line: row.get::<_, i64>(5)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
        };

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
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
        let all_classes = self.find_classes("")?;
        let class_map = classes_by_name(&all_classes);
        let Some(target) = class_map
            .get(class_name)
            .and_then(|classes| classes.first())
            .cloned()
        else {
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
                if let Some(parent) = class_map
                    .get(parent_name)
                    .and_then(|classes| classes.first())
                    .cloned()
                {
                    result.push(parent.clone());
                    queue.push_back(parent);
                }
            }
        }
        Ok(result)
    }

    pub(crate) fn find_sub_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let all_classes = self.find_classes("")?;
        let mut children_by_parent: HashMap<String, Vec<ClassInfo>> = HashMap::new();
        for class in all_classes {
            for parent in &class.super_classes {
                children_by_parent
                    .entry(parent.clone())
                    .or_default()
                    .push(class.clone());
            }
        }

        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(class_name.to_string());
        queue.push_back(class_name.to_string());

        while let Some(current) = queue.pop_front() {
            for child in children_by_parent.get(&current).into_iter().flatten() {
                if visited.insert(child.name.clone()) {
                    queue.push_back(child.name.clone());
                    result.push(child.clone());
                }
            }
        }
        Ok(result)
    }

    fn load_class_methods_map(
        &self,
        class_ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, Vec<String>>> {
        if class_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = repeat_placeholders(class_ids.len());
        let sql = format!(
            "SELECT class_id, method_name FROM class_methods WHERE class_id IN ({placeholders}) ORDER BY class_id, method_name"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(class_ids.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut map: HashMap<i64, Vec<String>> = HashMap::new();
        for row in rows {
            let (class_id, method_name) = row?;
            map.entry(class_id).or_default().push(method_name);
        }
        Ok(map)
    }

    fn load_class_super_classes_map(
        &self,
        class_ids: &[i64],
    ) -> anyhow::Result<HashMap<i64, Vec<String>>> {
        if class_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = repeat_placeholders(class_ids.len());
        let sql = format!(
            "SELECT class_id, super_class_name FROM class_super_classes WHERE class_id IN ({placeholders}) ORDER BY class_id, super_class_name"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(class_ids.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut map: HashMap<i64, Vec<String>> = HashMap::new();
        for row in rows {
            let (class_id, super_class_name) = row?;
            map.entry(class_id).or_default().push(super_class_name);
        }
        Ok(map)
    }

    fn load_field_names_map(
        &self,
        file_class_pairs: &[(i64, String)],
    ) -> anyhow::Result<HashMap<(i64, String), Vec<String>>> {
        if file_class_pairs.is_empty() {
            return Ok(HashMap::new());
        }

        let predicates =
            std::iter::repeat_n("(file_id = ? AND class_name = ?)", file_class_pairs.len())
                .collect::<Vec<_>>()
                .join(" OR ");
        let sql = format!(
            "SELECT file_id, class_name, name FROM fields WHERE {predicates} ORDER BY file_id, class_name, start_line"
        );

        let mut bind_values: Vec<&dyn ToSql> = Vec::with_capacity(file_class_pairs.len() * 2);
        for (file_id, class_name) in file_class_pairs {
            bind_values.push(file_id);
            bind_values.push(class_name);
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(bind_values), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        let mut map: HashMap<(i64, String), Vec<String>> = HashMap::new();
        for row in rows {
            let (file_id, class_name, field_name) = row?;
            map.entry((file_id, class_name))
                .or_default()
                .push(field_name);
        }
        Ok(map)
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
        let language = self.requested_language.as_deref();
        if class_name.is_some() {
            sql.push_str(" AND fn.class_name = ?2");
        }
        if language.is_some() {
            sql.push_str(if class_name.is_some() {
                " AND f.language = ?3"
            } else {
                " AND f.language = ?2"
            });
        }
        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<i64> = match (class_name, language) {
            (Some(class_name), Some(language)) => stmt
                .query_map(params![function_name, class_name, language], |row| {
                    row.get::<_, i64>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(class_name), None) => stmt
                .query_map(params![function_name, class_name], |row| {
                    row.get::<_, i64>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(language)) => stmt
                .query_map(params![function_name, language], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([function_name], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows.into_iter().collect())
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
        let language = self.requested_language.as_deref();
        if class_name.is_some() {
            sql.push_str(" AND pp.class_name = ?2");
        }
        if language.is_some() {
            sql.push_str(if class_name.is_some() {
                " AND f.language = ?3"
            } else {
                " AND f.language = ?2"
            });
        }
        let mut stmt = self.conn.prepare(&sql)?;
        let exists = match (class_name, language) {
            (Some(class_name), Some(language)) => stmt
                .query_row(params![function_name, class_name, language], |_| Ok(()))
                .optional()?,
            (Some(class_name), None) => stmt
                .query_row(params![function_name, class_name], |_| Ok(()))
                .optional()?,
            (None, Some(language)) => stmt
                .query_row(params![function_name, language], |_| Ok(()))
                .optional()?,
            (None, None) => stmt.query_row([function_name], |_| Ok(())).optional()?,
        };
        Ok(exists.is_some())
    }

    fn matches_call_target_class(
        &self,
        row: &CallLookupRow,
        class_name: &str,
        field_type_cache: &mut FieldTypeCache,
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

        let field_types =
            self.field_types_for_class(row.file_id, caller_class_name, field_type_cache)?;
        for field_type in field_types.get(&attr_name).into_iter().flatten() {
            if type_matches_class(field_type.as_deref(), class_name) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn field_types_for_class(
        &self,
        file_id: i64,
        class_name: &str,
        cache: &mut FieldTypeCache,
    ) -> anyhow::Result<FieldTypesByName> {
        let key = (file_id, class_name.to_string());
        if let Some(cached) = cache.get(&key) {
            return Ok(cached.clone());
        }

        let mut stmt = self.conn.prepare(
            "
            SELECT name, field_type
            FROM fields
            WHERE file_id = ?1 AND class_name = ?2
            ",
        )?;
        let rows = stmt.query_map(params![file_id, class_name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut map: FieldTypesByName = HashMap::new();
        for row in rows {
            let (name, field_type) = row?;
            map.entry(name).or_default().push(field_type);
        }
        cache.insert(key, map.clone());
        Ok(map)
    }
}
