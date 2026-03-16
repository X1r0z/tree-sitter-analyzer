use std::collections::HashSet;

use rusqlite::params;

use super::{LookupQuery, QueryContext};
use crate::models::ClassInfo;

pub(crate) struct ClassHierarchyQuery<'a> {
    ctx: QueryContext<'a>,
}

impl<'a> ClassHierarchyQuery<'a> {
    pub(crate) fn new(ctx: QueryContext<'a>) -> Self {
        Self { ctx }
    }

    pub(crate) fn find_super_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let seed_ids = self.load_seed_class_ids(class_name)?;
        if seed_ids.is_empty() {
            return Ok(Vec::new());
        }

        let reachable_ids = self.load_super_class_ids(class_name, &seed_ids)?;
        LookupQuery::new(self.ctx).load_classes_by_ids(&reachable_ids)
    }

    pub(crate) fn find_sub_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let seed_ids = self.load_seed_class_ids(class_name)?;
        if seed_ids.is_empty() {
            return Ok(Vec::new());
        }

        let reachable_ids = self.load_sub_class_ids(class_name, &seed_ids)?;
        LookupQuery::new(self.ctx).load_classes_by_ids(&reachable_ids)
    }

    fn load_seed_class_ids(&self, class_name: &str) -> anyhow::Result<Vec<i64>> {
        let sql = if self.ctx.language.is_some() {
            "
            SELECT c.id
            FROM classes c
            JOIN files f ON f.id = c.file_id
            WHERE c.name = ?1 AND f.language = ?2
            ORDER BY f.path, c.start_line
            "
        } else {
            "
            SELECT c.id
            FROM classes c
            JOIN files f ON f.id = c.file_id
            WHERE c.name = ?1
            ORDER BY f.path, c.start_line
            "
        };
        let mut stmt = self.ctx.conn.prepare(sql)?;
        match self.ctx.language {
            Some(language) => stmt
                .query_map(params![class_name, language], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into),
            None => stmt
                .query_map(params![class_name], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into),
        }
    }

    fn load_super_class_ids(&self, class_name: &str, seed_ids: &[i64]) -> anyhow::Result<Vec<i64>> {
        let sql = if self.ctx.language.is_some() {
            "
            WITH RECURSIVE
                seed(id) AS (
                    SELECT c.id
                    FROM classes c
                    JOIN files f ON f.id = c.file_id
                    WHERE c.name = ?1 AND f.language = ?2
                ),
                reachable(id) AS (
                    SELECT id FROM seed
                    UNION
                    SELECT parent.id
                    FROM reachable cur
                    JOIN class_super_classes rel ON rel.class_id = cur.id
                    JOIN classes parent ON parent.name = rel.super_class_name
                    JOIN files f ON f.id = parent.file_id
                    WHERE f.language = ?2
                )
            SELECT DISTINCT id
            FROM reachable
            "
        } else {
            "
            WITH RECURSIVE
                seed(id) AS (
                    SELECT c.id
                    FROM classes c
                    JOIN files f ON f.id = c.file_id
                    WHERE c.name = ?1
                ),
                reachable(id) AS (
                    SELECT id FROM seed
                    UNION
                    SELECT parent.id
                    FROM reachable cur
                    JOIN class_super_classes rel ON rel.class_id = cur.id
                    JOIN classes parent ON parent.name = rel.super_class_name
                )
            SELECT DISTINCT id
            FROM reachable
            "
        };
        let mut stmt = self.ctx.conn.prepare(sql)?;
        let reachable_ids = match self.ctx.language {
            Some(language) => stmt
                .query_map(params![class_name, language], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?,
            None => stmt
                .query_map(params![class_name], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?,
        };
        self.collect_non_seed_ids(reachable_ids, seed_ids)
    }

    fn load_sub_class_ids(&self, class_name: &str, seed_ids: &[i64]) -> anyhow::Result<Vec<i64>> {
        let sql = if self.ctx.language.is_some() {
            "
            WITH RECURSIVE
                seed(id, name) AS (
                    SELECT c.id, c.name
                    FROM classes c
                    JOIN files f ON f.id = c.file_id
                    WHERE c.name = ?1 AND f.language = ?2
                ),
                reachable(id, name) AS (
                    SELECT id, name FROM seed
                    UNION
                    SELECT child.id, child.name
                    FROM reachable cur
                    JOIN class_super_classes rel ON rel.super_class_name = cur.name
                    JOIN classes child ON child.id = rel.class_id
                    JOIN files f ON f.id = child.file_id
                    WHERE f.language = ?2
                )
            SELECT DISTINCT id
            FROM reachable
            "
        } else {
            "
            WITH RECURSIVE
                seed(id, name) AS (
                    SELECT c.id, c.name
                    FROM classes c
                    JOIN files f ON f.id = c.file_id
                    WHERE c.name = ?1
                ),
                reachable(id, name) AS (
                    SELECT id, name FROM seed
                    UNION
                    SELECT child.id, child.name
                    FROM reachable cur
                    JOIN class_super_classes rel ON rel.super_class_name = cur.name
                    JOIN classes child ON child.id = rel.class_id
                )
            SELECT DISTINCT id
            FROM reachable
            "
        };
        let mut stmt = self.ctx.conn.prepare(sql)?;
        let reachable_ids = match self.ctx.language {
            Some(language) => stmt
                .query_map(params![class_name, language], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?,
            None => stmt
                .query_map(params![class_name], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?,
        };
        self.collect_non_seed_ids(reachable_ids, seed_ids)
    }

    fn collect_non_seed_ids(
        &self,
        reachable_ids: Vec<i64>,
        seed_ids: &[i64],
    ) -> anyhow::Result<Vec<i64>> {
        let seed_ids: HashSet<i64> = seed_ids.iter().copied().collect();
        Ok(reachable_ids
            .into_iter()
            .filter(|id| !seed_ids.contains(id))
            .collect())
    }
}
