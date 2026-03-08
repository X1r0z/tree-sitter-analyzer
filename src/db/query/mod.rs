use rusqlite::Connection;

mod call_graph;
mod call_relations;
mod call_resolution;
mod catalog;
mod class_hierarchy;
mod prefilter;

pub(crate) use call_graph::CallGraphQuery;
pub(crate) use call_relations::CallRelationQuery;
pub(crate) use catalog::CatalogQuery;
pub(crate) use class_hierarchy::ClassHierarchyQuery;

#[derive(Clone, Copy)]
pub(crate) struct DbQueryContext<'a> {
    pub(crate) conn: &'a Connection,
    pub(crate) language: Option<&'a str>,
}
