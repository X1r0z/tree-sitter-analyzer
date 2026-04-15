use rusqlite::Connection;

use crate::db::IndexStore;
use crate::models::*;
use crate::query::{CallEdgeQuery, CallGraphQuery, ClassHierarchyQuery, LookupQuery, QueryContext};

pub(crate) struct StoreAnalyzer {
    conn: Connection,
    language: Option<String>,
}

impl StoreAnalyzer {
    pub(crate) fn from_current_dir(
        root_path: &str,
        language: Option<&str>,
    ) -> anyhow::Result<Self> {
        let db_path = std::env::current_dir()?.join("tsa.db");
        let store = IndexStore::open(&db_path)?;
        anyhow::ensure!(
            store.is_compatible_with(root_path, language)?,
            "Index at '{}' is incompatible with path/language",
            db_path.display()
        );
        Ok(Self {
            conn: store.into_connection(),
            language: language.map(str::to_string),
        })
    }

    pub(crate) fn file_count(&self) -> usize {
        LookupQuery::new(self.query_context()).file_count()
    }

    pub(crate) fn find_functions(&self, query: &str) -> anyhow::Result<Vec<FunctionInfo>> {
        LookupQuery::new(self.query_context()).find_functions(query)
    }

    pub(crate) fn find_classes(&self, query: &str) -> anyhow::Result<Vec<ClassInfo>> {
        LookupQuery::new(self.query_context()).find_classes(query)
    }

    pub(crate) fn find_fields(&self, class_name: &str) -> anyhow::Result<Vec<FieldInfo>> {
        LookupQuery::new(self.query_context()).find_fields(class_name)
    }

    pub(crate) fn find_imports(&self, query: &str) -> anyhow::Result<Vec<ImportInfo>> {
        LookupQuery::new(self.query_context()).find_imports(query)
    }

    pub(crate) fn find_annotations(&self, query: &str) -> anyhow::Result<Vec<AnnotationInfo>> {
        LookupQuery::new(self.query_context()).find_annotations(query)
    }

    pub(crate) fn find_refs(&self, name: &str) -> anyhow::Result<Vec<RefInfo>> {
        LookupQuery::new(self.query_context()).find_refs(name)
    }

    pub(crate) fn find_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CallerInfo>> {
        CallEdgeQuery::new(self.query_context()).find_callers(function_name, class_name)
    }

    pub(crate) fn find_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> anyhow::Result<Vec<CalleeInfo>> {
        CallEdgeQuery::new(self.query_context()).find_callees(function_name, class_name)
    }

    pub(crate) fn find_graphs(
        &self,
        function_name: &str,
        class_name: Option<&str>,
        direction: GraphDirection,
        max_depth: usize,
    ) -> anyhow::Result<Vec<CallGraphPath>> {
        CallGraphQuery::new(self.query_context()).find_graphs(
            function_name,
            class_name,
            direction,
            max_depth,
        )
    }

    pub(crate) fn find_super_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        ClassHierarchyQuery::new(self.query_context()).find_super_classes(class_name)
    }

    pub(crate) fn find_sub_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        ClassHierarchyQuery::new(self.query_context()).find_sub_classes(class_name)
    }

    fn query_context(&self) -> QueryContext<'_> {
        QueryContext {
            conn: &self.conn,
            language: self.language.as_deref(),
        }
    }
}
