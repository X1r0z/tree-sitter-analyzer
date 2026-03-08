use std::collections::HashMap;

use rusqlite::{params, OptionalExtension};

use super::call_relations::DbFunctionNode;
use super::DbQueryContext;
use crate::utils::{extract_instance_attr, type_matches_class};

pub(super) type FieldTypesByName = HashMap<String, Vec<Option<String>>>;
pub(super) type FieldTypeCache = HashMap<(i64, String), FieldTypesByName>;
pub(super) type ParamTypesByName = HashMap<String, Vec<Option<String>>>;
pub(super) type ParamTypeCache = HashMap<i64, ParamTypesByName>;

pub(crate) struct CallTargetResolver<'a> {
    ctx: DbQueryContext<'a>,
}

impl<'a> CallTargetResolver<'a> {
    pub(crate) fn new(ctx: DbQueryContext<'a>) -> Self {
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
        caller: Option<&DbFunctionNode>,
        object_name: Option<&str>,
        class_name: &str,
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<bool> {
        if object_name == Some(class_name) {
            return Ok(true);
        }
        if object_name
            .and_then(|object_name| object_name.split('(').next())
            .and_then(|head| head.rsplit('.').next())
            .is_some_and(|name| name == class_name)
        {
            return Ok(true);
        }

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
            let field_types = self.load_field_types_by_file_class(
                caller.file_id,
                caller_class_name,
                field_type_cache,
            )?;
            for field_type in field_types.get(attr_name).into_iter().flatten() {
                if type_matches_class(field_type.as_deref(), class_name) {
                    return Ok(true);
                }
            }
        }

        let param_types =
            self.load_param_types_by_function(caller.function_id, param_type_cache)?;
        for param_type in param_types.get(attr_name).into_iter().flatten() {
            if type_matches_class(param_type.as_deref(), class_name) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub(super) fn resolve_forward_targets(
        &self,
        caller: &DbFunctionNode,
        object_name: Option<&str>,
        candidates: &[DbFunctionNode],
        field_type_cache: &mut FieldTypeCache,
        param_type_cache: &mut ParamTypeCache,
    ) -> anyhow::Result<Vec<DbFunctionNode>> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

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
                    let field_types = self.load_field_types_by_file_class(
                        caller.file_id,
                        class_name,
                        field_type_cache,
                    )?;
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
                        self.load_param_types_by_function(caller.function_id, param_type_cache)?;
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
