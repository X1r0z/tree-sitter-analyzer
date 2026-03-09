use rusqlite::Connection;

mod call_edges;
mod call_graph;
mod call_resolver;
mod class_hierarchy;
mod lookup;
mod prefilter;

pub(crate) use call_edges::CallEdgeQuery;
pub(crate) use call_graph::CallGraphQuery;
pub(crate) use class_hierarchy::ClassHierarchyQuery;
pub(crate) use lookup::LookupQuery;

#[derive(Clone, Copy)]
pub(crate) struct QueryContext<'a> {
    pub(crate) conn: &'a Connection,
    pub(crate) language: Option<&'a str>,
}
