use std::collections::{HashMap, HashSet};

use rusqlite::{params, params_from_iter, OptionalExtension, ToSql};

use super::prefilter::RegexPrefilter;
use super::types::{
    classes_by_name, CallLookupRow, CalleeLookupRow, ClassBaseRow, DbFunctionNode, FieldTypeCache,
    FieldTypesByName, ParamTypeCache, ParamTypesByName,
};
use super::DbProjectAnalyzer;
use crate::models::{
    AnnotationInfo, CallGraphPath, CalleeInfo, CallerInfo, ClassInfo, FieldInfo, FunctionInfo,
    FunctionKey, GraphDirection, GraphPathNode, ImportInfo, Location, SymbolRefInfo,
};
use crate::utils::{
    extract_instance_attr, sort_callees_by_file_line, sort_callers_by_file_line,
    split_function_target, type_matches_class,
};
use crate::walk::bfs::collect_reachable;
use crate::walk::dfs::{try_collect_paths, PathStep};

#[derive(Clone)]
struct CallSite {
    file: String,
    line: usize,
}

#[derive(Clone)]
struct GraphNeighbor {
    node: DbFunctionNode,
    call_site: CallSite,
}

struct GraphTraversalState {
    neighbor_cache: HashMap<(GraphDirection, FunctionKey), Vec<GraphNeighbor>>,
    node_cache: HashMap<(String, Option<String>), Vec<DbFunctionNode>>,
    field_type_cache: FieldTypeCache,
    param_type_cache: ParamTypeCache,
    is_property_cache: HashMap<(String, Option<String>), bool>,
}

const SQLITE_BATCH_SIZE: usize = 256;

fn repeat_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

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
        let prefilter = RegexPrefilter::new(query);
        let mut sql = String::from(
            "
            SELECT f.path, fn.name, fn.class_name, fn.start_line, fn.end_line
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
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
            params.push(language);
        }
        sql.push_str(" ORDER BY f.path, fn.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(FunctionInfo {
                name: row.get(1)?,
                location: Location {
                    file: row.get(0)?,
                    start_line: row.get::<_, i64>(3)? as usize,
                    end_line: row.get::<_, i64>(4)? as usize,
                },
                body: String::new(),
                class_name: row.get(2)?,
                params: Vec::new(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn find_classes(&self, query: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let prefilter = RegexPrefilter::new(query);
        let mut sql = String::from(
            "
            SELECT c.id, c.file_id, f.path, c.name, c.start_line, c.end_line
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
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
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
        let class_rows = rows.collect::<Result<Vec<_>, _>>()?;
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
        let prefilter = RegexPrefilter::new(query);
        let mut sql = String::from(
            "
            SELECT f.path, i.module, i.start_line
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
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
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
        if let Some(language) = self.requested_language.as_ref() {
            sql.push_str(&format!(" AND f.language = ?{}", params.len() + 1));
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
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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
    ) -> anyhow::Result<Vec<CallerInfo>> {
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
        let mut param_type_cache = HashMap::new();
        let mut node_cache = HashMap::new();
        for row in rows {
            let row = row?;
            if let Some(target_object) = target_object {
                if row.object_name.as_deref() != Some(target_object) {
                    continue;
                }
            }
            if let Some(class_name) = class_name {
                let Some(caller_name) = row.caller.as_deref() else {
                    continue;
                };
                let Some(caller) = self.resolve_enclosing_db_function(
                    row.file_id,
                    &row.file,
                    caller_name,
                    row.caller_class_name.as_deref(),
                    row.line,
                    &mut node_cache,
                )?
                else {
                    continue;
                };
                if !self.matches_call_target_for_caller(
                    &caller,
                    row.object_name.as_deref(),
                    class_name,
                    &mut field_type_cache,
                    &mut param_type_cache,
                )? {
                    continue;
                }
            }
            let caller = row.caller.unwrap_or_else(|| "<module>".to_string());
            let key = (row.file.clone(), caller.clone(), row.line);
            if seen.insert(key) {
                results.push(CallerInfo {
                    caller,
                    line: row.line,
                    file: row.file,
                });
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
                    results.push(CallerInfo { caller, line, file });
                }
            }
        }

        sort_callers_by_file_line(&mut results);
        Ok(results)
    }

    pub(crate) fn find_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CalleeInfo>> {
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
                results.push(CalleeInfo {
                    callee: callee_name,
                    line: row.line,
                    file: row.file,
                    class_name: row.caller_class_name,
                });
            }
        }
        sort_callees_by_file_line(&mut results);
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

        Ok(collect_reachable(
            [target],
            [class_name.to_string()],
            |current| {
                current
                    .super_classes
                    .iter()
                    .filter_map(|parent_name| {
                        class_map
                            .get(parent_name)
                            .and_then(|classes| classes.first())
                            .cloned()
                    })
                    .collect()
            },
            |parent| parent.clone(),
            |parent| parent.name.clone(),
        ))
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

        Ok(collect_reachable(
            [class_name.to_string()],
            [class_name.to_string()],
            |current| children_by_parent.get(current).cloned().unwrap_or_default(),
            |child| child.name.clone(),
            |child| child.name.clone(),
        ))
    }

    pub(crate) fn find_graphs(
        &self,
        function_name: &str,
        class_name: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
    ) -> anyhow::Result<Vec<CallGraphPath>> {
        let start_nodes = self.find_exact_function_nodes(function_name, class_name)?;
        if start_nodes.is_empty() {
            anyhow::bail!("Function '{}' not found", function_name);
        }

        let mut state = GraphTraversalState {
            neighbor_cache: HashMap::new(),
            node_cache: HashMap::new(),
            field_type_cache: HashMap::new(),
            param_type_cache: HashMap::new(),
            is_property_cache: HashMap::new(),
        };

        let neighbor_cache = &mut state.neighbor_cache;
        let node_cache = &mut state.node_cache;
        let field_type_cache = &mut state.field_type_cache;
        let param_type_cache = &mut state.param_type_cache;
        let is_property_cache = &mut state.is_property_cache;

        let mut results = try_collect_paths(
            &start_nodes,
            direction,
            max_depth,
            DbFunctionNode::key,
            |current| {
                let cache_key = (direction, current.key());
                let neighbors = if let Some(cached) = neighbor_cache.get(&cache_key) {
                    cached.clone()
                } else {
                    let loaded = self.load_graph_neighbors(
                        current,
                        direction,
                        node_cache,
                        field_type_cache,
                        param_type_cache,
                        is_property_cache,
                    )?;
                    neighbor_cache.insert(cache_key, loaded.clone());
                    loaded
                };

                Ok::<Vec<(DbFunctionNode, CallSite)>, anyhow::Error>(
                    neighbors
                        .into_iter()
                        .map(|neighbor| (neighbor.node, neighbor.call_site))
                        .collect(),
                )
            },
            Self::materialize_graph,
        )?;

        results.sort_by(|left, right| {
            left.stacktrace
                .cmp(&right.stacktrace)
                .then(left.depth.cmp(&right.depth))
                .then(left.path.len().cmp(&right.path.len()))
        });
        Ok(results)
    }

    fn load_class_methods_map(
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
            let mut stmt = self.conn.prepare(&sql)?;
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

    fn materialize_graph(
        direction: GraphDirection,
        steps: &[PathStep<DbFunctionNode, CallSite>],
    ) -> CallGraphPath {
        let path: Vec<GraphPathNode> = steps
            .iter()
            .map(|step| GraphPathNode::from(&step.node.function))
            .collect();
        let stacktrace = direction.order_stacktrace(
            path.iter()
                .zip(steps.iter())
                .map(|(node, step)| {
                    let (file, line) = step
                        .edge
                        .as_ref()
                        .map(|call_site| (call_site.file.as_str(), call_site.line))
                        .unwrap_or((node.file.as_str(), node.start_line));
                    node.stacktrace_name(file, line)
                })
                .collect(),
        );
        CallGraphPath {
            depth: path.len().saturating_sub(1),
            stacktrace,
            path,
        }
    }

    fn load_graph_neighbors(
        &self,
        node: &DbFunctionNode,
        direction: GraphDirection,
        node_cache: &mut HashMap<(String, Option<String>), Vec<DbFunctionNode>>,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
        is_property_cache: &mut HashMap<(String, Option<String>), bool>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        match direction {
            GraphDirection::Forward => {
                self.load_forward_neighbors(node, node_cache, field_type_cache, param_type_cache)
            }
            GraphDirection::Backward => self.load_backward_neighbors(
                node,
                node_cache,
                field_type_cache,
                param_type_cache,
                is_property_cache,
            ),
        }
    }

    fn load_forward_neighbors(
        &self,
        node: &DbFunctionNode,
        node_cache: &mut HashMap<(String, Option<String>), Vec<DbFunctionNode>>,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let start_line = node.function.location.start_line as i64;
        let end_line = node.function.location.end_line as i64;
        let mut sql = String::from(
            "
            SELECT callee, object_name, start_line
            FROM calls
            WHERE file_id = ?1 AND caller = ?2 AND start_line >= ?3 AND start_line <= ?4
            ",
        );

        let rows = match node.function.class_name.as_deref() {
            Some(class_name) => {
                sql.push_str(" AND caller_class_name = ?5");
                sql.push_str(" ORDER BY start_line");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(
                    params![
                        node.file_id,
                        node.function.name,
                        start_line,
                        end_line,
                        class_name
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, i64>(2)? as usize,
                        ))
                    },
                )?;
                rows.collect::<Result<Vec<_>, _>>()?
            }
            None => {
                sql.push_str(" AND caller_class_name IS NULL");
                sql.push_str(" ORDER BY start_line");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(
                    params![node.file_id, node.function.name, start_line, end_line],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, i64>(2)? as usize,
                        ))
                    },
                )?;
                rows.collect::<Result<Vec<_>, _>>()?
            }
        };

        let mut seen = HashSet::new();
        let mut results = Vec::new();
        for (callee_name, object_name, line) in rows {
            let candidates = self.load_function_nodes_by_name(&callee_name, node_cache)?;
            for candidate in self.resolve_down_targets(
                node,
                object_name.as_deref(),
                &candidates,
                field_type_cache,
                param_type_cache,
            )? {
                let key = candidate.key();
                if seen.insert(key) {
                    results.push(GraphNeighbor {
                        node: candidate,
                        call_site: CallSite {
                            file: node.function.location.file.clone(),
                            line,
                        },
                    });
                }
            }
        }
        Ok(results)
    }

    fn load_backward_neighbors(
        &self,
        node: &DbFunctionNode,
        node_cache: &mut HashMap<(String, Option<String>), Vec<DbFunctionNode>>,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
        is_property_cache: &mut HashMap<(String, Option<String>), bool>,
    ) -> anyhow::Result<Vec<GraphNeighbor>> {
        let mut sql = String::from(
            "
            SELECT c.file_id, f.path, c.caller, c.caller_class_name, c.object_name, c.start_line
            FROM calls c
            JOIN files f ON f.id = c.file_id
            WHERE c.callee = ?1
            ",
        );
        let mut params: Vec<&dyn ToSql> = vec![&node.function.name];
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

        let mut seen = HashSet::new();
        let mut results = Vec::new();
        for row in rows {
            let row = row?;
            let Some(caller_name) = row.caller.as_deref() else {
                continue;
            };
            let caller = self.resolve_enclosing_db_function(
                row.file_id,
                &row.file,
                caller_name,
                row.caller_class_name.as_deref(),
                row.line,
                node_cache,
            )?;
            if let Some(caller) = caller {
                if let Some(class_name) = node.function.class_name.as_deref() {
                    if !self.matches_call_target_for_caller(
                        &caller,
                        row.object_name.as_deref(),
                        class_name,
                        field_type_cache,
                        param_type_cache,
                    )? {
                        continue;
                    }
                }
                let key = caller.key();
                if seen.insert(key) {
                    results.push(GraphNeighbor {
                        node: caller,
                        call_site: CallSite {
                            file: row.file.clone(),
                            line: row.line,
                        },
                    });
                }
            }
        }

        let property_key = (node.function.name.clone(), node.function.class_name.clone());
        let is_property = if let Some(value) = is_property_cache.get(&property_key) {
            *value
        } else {
            let value =
                self.is_python_property(&node.function.name, node.function.class_name.as_deref())?;
            is_property_cache.insert(property_key.clone(), value);
            value
        };

        if is_property {
            let mut property_sql = String::from(
                "
                SELECT ppc.file_id, f.path, ppc.caller, ppc.line
                FROM python_property_callers ppc
                JOIN files f ON f.id = ppc.file_id
                WHERE ppc.property_name = ?1
                ",
            );
            let mut property_params: Vec<&dyn ToSql> = vec![&node.function.name];
            if let Some(language) = self.requested_language.as_ref() {
                property_sql.push_str(" AND f.language = ?2");
                property_params.push(language);
            }
            property_sql.push_str(" ORDER BY f.path, ppc.line");

            let mut property_stmt = self.conn.prepare(&property_sql)?;
            let property_rows =
                property_stmt.query_map(params_from_iter(property_params), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)? as usize,
                    ))
                })?;

            for row in property_rows {
                let (file_id, file, caller_name, line) = row?;
                if let Some(caller) = self.resolve_enclosing_db_function(
                    file_id,
                    &file,
                    &caller_name,
                    None,
                    line,
                    node_cache,
                )? {
                    let key = caller.key();
                    if seen.insert(key) {
                        results.push(GraphNeighbor {
                            node: caller,
                            call_site: CallSite { file, line },
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    fn resolve_down_targets(
        &self,
        caller: &DbFunctionNode,
        object_name: Option<&str>,
        candidates: &[DbFunctionNode],
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<Vec<DbFunctionNode>> {
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        match object_name {
            Some("self") | Some("this") | Some("cls") => {
                if let Some(class_name) = caller.function.class_name.as_deref() {
                    for candidate in candidates {
                        if candidate.function.class_name.as_deref() == Some(class_name)
                            && seen.insert(candidate.key())
                        {
                            results.push(candidate.clone());
                        }
                    }
                }
            }
            Some(object_name) => {
                for candidate in candidates {
                    if candidate.function.class_name.as_deref() == Some(object_name)
                        && seen.insert(candidate.key())
                    {
                        results.push(candidate.clone());
                    }
                }

                if let (Some(class_name), Some(attr_name)) = (
                    caller.function.class_name.as_deref(),
                    extract_instance_attr(object_name),
                ) {
                    let field_types =
                        self.field_types_for_class(caller.file_id, class_name, field_type_cache)?;
                    for candidate in candidates {
                        for field_type in field_types.get(attr_name).into_iter().flatten() {
                            if type_matches_class(
                                field_type.as_deref(),
                                candidate.function.class_name.as_deref().unwrap_or_default(),
                            ) && seen.insert(candidate.key())
                            {
                                results.push(candidate.clone());
                            }
                        }
                    }
                }

                if let Some(param_name) = extract_instance_attr(object_name) {
                    let param_types =
                        self.param_types_for_function(caller.function_id, param_type_cache)?;
                    for candidate in candidates {
                        for param_type in param_types.get(param_name).into_iter().flatten() {
                            if type_matches_class(
                                param_type.as_deref(),
                                candidate.function.class_name.as_deref().unwrap_or_default(),
                            ) && seen.insert(candidate.key())
                            {
                                results.push(candidate.clone());
                            }
                        }
                    }
                }
            }
            None => {
                if let Some(class_name) = caller.function.class_name.as_deref() {
                    for candidate in candidates {
                        if candidate.function.class_name.as_deref() == Some(class_name)
                            && seen.insert(candidate.key())
                        {
                            results.push(candidate.clone());
                        }
                    }
                }

                let same_file_globals: Vec<_> = candidates
                    .iter()
                    .filter(|candidate| {
                        candidate.function.class_name.is_none()
                            && candidate.function.location.file == caller.function.location.file
                    })
                    .cloned()
                    .collect();

                if same_file_globals.is_empty() {
                    for candidate in candidates {
                        if candidate.function.class_name.is_none() && seen.insert(candidate.key()) {
                            results.push(candidate.clone());
                        }
                    }
                } else {
                    for candidate in same_file_globals {
                        if seen.insert(candidate.key()) {
                            results.push(candidate);
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    fn find_exact_function_nodes(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<DbFunctionNode>> {
        let mut sql = String::from(
            "
            SELECT fn.id, fn.file_id, f.path, fn.name, fn.class_name, fn.start_line, fn.end_line
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
        sql.push_str(" ORDER BY f.path, fn.start_line");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<DbFunctionNode> = match (class_name, language) {
            (Some(class_name), Some(language)) => stmt
                .query_map(params![function_name, class_name, language], |row| {
                    Self::row_to_db_function_node(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(class_name), None) => stmt
                .query_map(params![function_name, class_name], |row| {
                    Self::row_to_db_function_node(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(language)) => stmt
                .query_map(params![function_name, language], |row| {
                    Self::row_to_db_function_node(row)
                })?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map([function_name], Self::row_to_db_function_node)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    fn load_function_nodes_by_name(
        &self,
        function_name: &str,
        cache: &mut HashMap<(String, Option<String>), Vec<DbFunctionNode>>,
    ) -> anyhow::Result<Vec<DbFunctionNode>> {
        let key = (function_name.to_string(), None);
        if let Some(cached) = cache.get(&key) {
            return Ok(cached.clone());
        }

        let loaded = self.find_exact_function_nodes(function_name, None)?;
        cache.insert(key, loaded.clone());
        Ok(loaded)
    }

    fn resolve_enclosing_db_function(
        &self,
        file_id: i64,
        file: &str,
        function_name: &str,
        class_name: Option<&str>,
        line: usize,
        cache: &mut HashMap<(String, Option<String>), Vec<DbFunctionNode>>,
    ) -> anyhow::Result<Option<DbFunctionNode>> {
        let cache_key = (function_name.to_string(), class_name.map(str::to_string));
        let candidates = if let Some(cached) = cache.get(&cache_key) {
            cached.clone()
        } else {
            let loaded = self.find_exact_function_nodes(function_name, class_name)?;
            cache.insert(cache_key.clone(), loaded.clone());
            loaded
        };

        Ok(candidates.into_iter().find(|candidate| {
            candidate.file_id == file_id
                && candidate.function.location.file == file
                && candidate.function.location.start_line <= line
                && line <= candidate.function.location.end_line
        }))
    }

    fn row_to_db_function_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<DbFunctionNode> {
        Ok(DbFunctionNode {
            function_id: row.get(0)?,
            file_id: row.get(1)?,
            function: FunctionInfo {
                name: row.get(3)?,
                location: Location {
                    file: row.get(2)?,
                    start_line: row.get::<_, i64>(5)? as usize,
                    end_line: row.get::<_, i64>(6)? as usize,
                },
                body: String::new(),
                class_name: row.get(4)?,
                params: Vec::new(),
            },
        })
    }

    fn load_class_super_classes_map(
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
            let mut stmt = self.conn.prepare(&sql)?;
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

    fn load_field_names_map(
        &self,
        file_class_pairs: &[(i64, String)],
    ) -> anyhow::Result<HashMap<(i64, String), Vec<String>>> {
        if file_class_pairs.is_empty() {
            return Ok(HashMap::new());
        }

        let mut map: HashMap<(i64, String), Vec<String>> = HashMap::new();
        for chunk in file_class_pairs.chunks(SQLITE_BATCH_SIZE) {
            let predicates = std::iter::repeat_n("(file_id = ? AND class_name = ?)", chunk.len())
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT file_id, class_name, name FROM fields WHERE {predicates} ORDER BY file_id, class_name, start_line"
            );

            let mut bind_values: Vec<&dyn ToSql> = Vec::with_capacity(chunk.len() * 2);
            for (file_id, class_name) in chunk {
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

            for row in rows {
                let (file_id, class_name, field_name) = row?;
                map.entry((file_id, class_name))
                    .or_default()
                    .push(field_name);
            }
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

    fn matches_call_target_for_caller(
        &self,
        caller: &DbFunctionNode,
        object_name: Option<&str>,
        class_name: &str,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<bool> {
        if object_name == Some(class_name) {
            return Ok(true);
        }
        if object_name.is_none() {
            return Ok(caller.function.class_name.as_deref() == Some(class_name));
        }
        if matches!(object_name, Some("self") | Some("this") | Some("cls")) {
            return Ok(caller.function.class_name.as_deref() == Some(class_name));
        }

        let Some(object_name) = object_name else {
            return Ok(false);
        };

        let Some(attr_name) = extract_instance_attr(object_name) else {
            return Ok(false);
        };

        if let Some(caller_class_name) = caller.function.class_name.as_deref() {
            let field_types =
                self.field_types_for_class(caller.file_id, caller_class_name, field_type_cache)?;
            for field_type in field_types.get(attr_name).into_iter().flatten() {
                if type_matches_class(field_type.as_deref(), class_name) {
                    return Ok(true);
                }
            }
        }

        let param_types = self.param_types_for_function(caller.function_id, param_type_cache)?;
        for param_type in param_types.get(attr_name).into_iter().flatten() {
            if type_matches_class(param_type.as_deref(), class_name) {
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

    fn param_types_for_function(
        &self,
        function_id: i64,
        cache: &mut ParamTypeCache,
    ) -> anyhow::Result<ParamTypesByName> {
        if let Some(cached) = cache.get(&function_id) {
            return Ok(cached.clone());
        }

        let mut stmt = self.conn.prepare(
            "
            SELECT name, param_type
            FROM function_params
            WHERE function_id = ?1
            ORDER BY position
            ",
        )?;
        let rows = stmt.query_map(params![function_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut map: ParamTypesByName = HashMap::new();
        for row in rows {
            let (name, param_type) = row?;
            map.entry(name).or_default().push(param_type);
        }
        cache.insert(function_id, map.clone());
        Ok(map)
    }
}
