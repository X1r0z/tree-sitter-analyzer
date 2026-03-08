use std::collections::HashMap;

use super::{CatalogQuery, DbQueryContext};
use crate::models::ClassInfo;
use crate::walk::bfs::collect_reachable;

pub(crate) struct ClassHierarchyQuery<'a> {
    ctx: DbQueryContext<'a>,
}

impl<'a> ClassHierarchyQuery<'a> {
    pub(crate) fn new(ctx: DbQueryContext<'a>) -> Self {
        Self { ctx }
    }

    pub(crate) fn find_super_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        let all_classes = CatalogQuery::new(self.ctx).find_classes("")?;
        let class_map = load_classes_by_name(&all_classes);
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
        let all_classes = CatalogQuery::new(self.ctx).find_classes("")?;
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
}

fn load_classes_by_name(classes: &[ClassInfo]) -> HashMap<String, Vec<ClassInfo>> {
    let mut map = HashMap::new();
    for class in classes {
        map.entry(class.name.clone())
            .or_insert_with(Vec::new)
            .push(class.clone());
    }
    map
}
