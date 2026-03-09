use std::collections::HashMap;

use rusqlite::{params, OptionalExtension};

use super::call_edges::IndexedFunction;
use super::QueryContext;
use crate::parser::call_targets::{
    has_unique_class_method_target, matches_call_target, resolve_forward_targets_with_fallback,
    type_matches_class, ForwardTargetContext,
};

pub(super) type FieldTypesByName = HashMap<String, Vec<Option<String>>>;
pub(super) type FieldTypeCache = HashMap<(i64, String), FieldTypesByName>;
pub(super) type ParamTypesByName = HashMap<String, Vec<Option<String>>>;
pub(super) type ParamTypeCache = HashMap<i64, ParamTypesByName>;

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
    ) -> anyhow::Result<bool> {
        let Some(caller) = caller else {
            return Ok(false);
        };
        self.matches_call_target(
            caller,
            object_name,
            class_name,
            field_type_cache,
            param_type_cache,
        )
    }

    pub(super) fn matches_call_target(
        &self,
        caller: &IndexedFunction,
        object_name: Option<&str>,
        class_name: &str,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<bool> {
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

    pub(super) fn resolve_forward_targets_with_fallback(
        &self,
        caller: &IndexedFunction,
        object_name: Option<&str>,
        candidates: &[IndexedFunction],
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<Vec<IndexedFunction>> {
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
    ) -> anyhow::Result<FieldTypesByName> {
        let key = (file_id, class_name.to_string());
        if let Some(cached) = cache.get(&key) {
            return Ok(cached.clone());
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
        cache.insert(key, map.clone());
        Ok(map)
    }

    fn load_param_types_by_function(
        &self,
        function_id: i64,
        cache: &mut ParamTypeCache,
    ) -> anyhow::Result<ParamTypesByName> {
        if let Some(cached) = cache.get(&function_id) {
            return Ok(cached.clone());
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
        cache.insert(function_id, map.clone());
        Ok(map)
    }
}
