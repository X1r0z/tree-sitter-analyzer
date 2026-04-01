use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::{params, OptionalExtension};

use super::call_edges::IndexedFunction;
use super::QueryContext;
use crate::parser::call_targets::{
    has_unique_class_method_target, matches_call_target,
    matches_property_target as call_matches_property_target, resolve_forward_targets_with_fallback,
    type_matches_class, ForwardTargetContext,
};
use crate::traversal::collect_reachable_bfs;

pub(super) type FieldTypesByName = HashMap<String, Vec<Option<String>>>;
pub(super) type FieldTypeCache = HashMap<(i64, String), Arc<FieldTypesByName>>;
pub(super) type ParamTypesByName = HashMap<String, Vec<Option<String>>>;
pub(super) type ParamTypeCache = HashMap<i64, Arc<ParamTypesByName>>;
pub(super) type AncestorsByFileClass = HashMap<(i64, String), Arc<[String]>>;

pub(crate) struct CallTargetResolver<'a> {
    ctx: QueryContext<'a>,
}

impl<'a> CallTargetResolver<'a> {
    pub(crate) fn new(ctx: QueryContext<'a>) -> Self {
        Self { ctx }
    }

    pub(super) fn is_python_property(
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
        let language = self.ctx.language;
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
        let mut stmt = self.ctx.conn.prepare(&sql)?;
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

    pub(super) fn matches_property_target(
        &self,
        caller: Option<&IndexedFunction>,
        object_name: Option<&str>,
        class_name: &str,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
        ancestors_cache: &mut AncestorsByFileClass,
    ) -> anyhow::Result<bool> {
        let Some(caller) = caller else {
            return Ok(false);
        };
        if matches!(object_name, Some("self") | Some("cls")) {
            let Some(caller_class_name) = caller.function.class_name.as_deref() else {
                return Ok(false);
            };
            return Ok(self
                .load_class_ancestors(caller.file_id, caller_class_name, ancestors_cache)?
                .iter()
                .any(|ancestor| ancestor == class_name)
                || caller_class_name == class_name);
        }
        let field_types = caller
            .function
            .class_name
            .as_deref()
            .map(|caller_class_name| {
                self.load_field_types_by_file_class(
                    caller.file_id,
                    caller_class_name,
                    field_type_cache,
                )
            })
            .transpose()?
            .unwrap_or_default();
        let param_types =
            self.load_param_types_by_function(caller.function_id, param_type_cache)?;
        Ok(call_matches_property_target(
            caller.function.class_name.as_deref(),
            object_name,
            class_name,
            |attr_name, target_class_name| {
                field_types
                    .get(attr_name)
                    .into_iter()
                    .flatten()
                    .any(|field_type| type_matches_class(field_type.as_deref(), target_class_name))
            },
            |attr_name, target_class_name| {
                param_types
                    .get(attr_name)
                    .into_iter()
                    .flatten()
                    .any(|param_type| type_matches_class(param_type.as_deref(), target_class_name))
            },
        ))
    }

    pub(super) fn matches_call_target(
        &self,
        caller: &IndexedFunction,
        object_name: Option<&str>,
        class_name: &str,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<bool> {
        if object_name == Some("super()") {
            let Some(caller_class_name) = caller.function.class_name.as_deref() else {
                return Ok(false);
            };
            return Ok(self
                .load_class_parents(caller.file_id, caller_class_name)?
                .iter()
                .any(|parent| parent == class_name));
        }
        let field_types = caller
            .function
            .class_name
            .as_deref()
            .map(|caller_class_name| {
                self.load_field_types_by_file_class(
                    caller.file_id,
                    caller_class_name,
                    field_type_cache,
                )
            })
            .transpose()?
            .unwrap_or_default();
        let param_types =
            self.load_param_types_by_function(caller.function_id, param_type_cache)?;
        Ok(matches_call_target(
            caller.function.class_name.as_deref(),
            object_name,
            class_name,
            |attr_name, target_class_name| {
                field_types
                    .get(attr_name)
                    .into_iter()
                    .flatten()
                    .any(|field_type| type_matches_class(field_type.as_deref(), target_class_name))
            },
            |attr_name, target_class_name| {
                param_types
                    .get(attr_name)
                    .into_iter()
                    .flatten()
                    .any(|param_type| type_matches_class(param_type.as_deref(), target_class_name))
            },
        ))
    }

    pub(super) fn matches_call_target_without_enclosing_function(
        &self,
        caller_class_name: Option<&str>,
        object_name: Option<&str>,
        class_name: &str,
    ) -> bool {
        matches_call_target(
            caller_class_name,
            object_name,
            class_name,
            |_attr_name, _target_class_name| false,
            |_attr_name, _target_class_name| false,
        )
    }

    pub(super) fn resolve_forward_targets_with_fallback(
        &self,
        caller: &IndexedFunction,
        object_name: Option<&str>,
        candidates: &[IndexedFunction],
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<Vec<IndexedFunction>> {
        if object_name == Some("super()") {
            let Some(caller_class_name) = caller.function.class_name.as_deref() else {
                return Ok(Vec::new());
            };
            let parents = self.load_class_parents(caller.file_id, caller_class_name)?;
            let mut resolved: Vec<_> = candidates
                .iter()
                .filter(|candidate| {
                    candidate
                        .function
                        .class_name
                        .as_deref()
                        .is_some_and(|class_name| parents.iter().any(|parent| parent == class_name))
                })
                .cloned()
                .collect();
            resolved.sort_by_key(|candidate| {
                (
                    candidate.function.location.file.clone(),
                    candidate.function.location.start_line,
                    candidate.function.location.end_line,
                    candidate.function.class_name.clone(),
                    candidate.function.name.clone(),
                )
            });
            resolved.dedup_by(|left, right| left.key() == right.key());
            return Ok(resolved);
        }
        let field_types = caller
            .function
            .class_name
            .as_deref()
            .map(|class_name| {
                self.load_field_types_by_file_class(caller.file_id, class_name, field_type_cache)
            })
            .transpose()?
            .unwrap_or_default();
        let param_types =
            self.load_param_types_by_function(caller.function_id, param_type_cache)?;
        Ok(resolve_forward_targets_with_fallback(
            ForwardTargetContext {
                caller_class_name: caller.function.class_name.as_deref(),
                caller_file: &caller.function.location.file,
                object_name,
            },
            candidates,
            IndexedFunction::key,
            |candidate| candidate.function.class_name.as_deref(),
            |candidate| candidate.function.location.file.as_str(),
            |attr_name, candidate_class_name| {
                field_types
                    .get(attr_name)
                    .into_iter()
                    .flatten()
                    .any(|field_type| {
                        type_matches_class(field_type.as_deref(), candidate_class_name)
                    })
            },
            |attr_name, candidate_class_name| {
                param_types
                    .get(attr_name)
                    .into_iter()
                    .flatten()
                    .any(|param_type| {
                        type_matches_class(param_type.as_deref(), candidate_class_name)
                    })
            },
        ))
    }

    pub(super) fn has_unique_method_target(
        &self,
        function_name: &str,
        class_name: &str,
    ) -> anyhow::Result<bool> {
        let mut sql = String::from(
            "
            SELECT DISTINCT fn.class_name
            FROM functions fn
            JOIN files f ON f.id = fn.file_id
            WHERE fn.name = ?1 AND fn.class_name IS NOT NULL
            ",
        );
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&function_name];
        if let Some(language) = self.ctx.language.as_ref() {
            sql.push_str(" AND f.language = ?2");
            params.push(language);
        }

        let mut stmt = self.ctx.conn.prepare(&sql)?;
        let classes = stmt
            .query_map(rusqlite::params_from_iter(params), |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(has_unique_class_method_target(
            &classes,
            class_name,
            |candidate| Some(candidate.as_str()),
        ))
    }

    fn load_field_types_by_file_class(
        &self,
        file_id: i64,
        class_name: &str,
        cache: &mut FieldTypeCache,
    ) -> anyhow::Result<Arc<FieldTypesByName>> {
        let key = (file_id, class_name.to_string());
        if let Some(cached) = cache.get(&key) {
            return Ok(Arc::clone(cached));
        }

        let mut stmt = self.ctx.conn.prepare(
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
        let map = Arc::new(map);
        cache.insert(key, Arc::clone(&map));
        Ok(map)
    }

    fn load_param_types_by_function(
        &self,
        function_id: i64,
        cache: &mut ParamTypeCache,
    ) -> anyhow::Result<Arc<ParamTypesByName>> {
        if let Some(cached) = cache.get(&function_id) {
            return Ok(Arc::clone(cached));
        }

        let mut stmt = self.ctx.conn.prepare(
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
        let map = Arc::new(map);
        cache.insert(function_id, Arc::clone(&map));
        Ok(map)
    }

    fn load_class_parents(&self, file_id: i64, class_name: &str) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.ctx.conn.prepare(
            "
            SELECT rel.super_class_name
            FROM classes cls
            JOIN class_super_classes rel ON rel.class_id = cls.id
            WHERE cls.file_id = ?1 AND cls.name = ?2
            ORDER BY rel.rowid
            ",
        )?;
        let rows = stmt.query_map(params![file_id, class_name], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn load_class_ancestors(
        &self,
        file_id: i64,
        class_name: &str,
        cache: &mut AncestorsByFileClass,
    ) -> anyhow::Result<Arc<[String]>> {
        let key = (file_id, class_name.to_string());
        if let Some(cached) = cache.get(&key) {
            return Ok(Arc::clone(cached));
        }
        let mut error = None;
        let ancestors = collect_reachable_bfs(
            [class_name.to_string()],
            [class_name.to_string()],
            |current| match self.load_class_parents(file_id, current) {
                Ok(parents) => parents,
                Err(err) => {
                    if error.is_none() {
                        error = Some(err);
                    }
                    Vec::new()
                }
            },
            Clone::clone,
            Clone::clone,
        );
        if let Some(err) = error {
            return Err(err);
        }
        let ordered: Arc<[String]> = Arc::from(ancestors);
        cache.insert(key, Arc::clone(&ordered));
        Ok(ordered)
    }
}
