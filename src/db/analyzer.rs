use rusqlite::Connection;

use super::query::{
    CallEdgeQuery, CallGraphQuery, ClassHierarchyQuery, LookupQuery, QueryContext,
};
use super::store::IndexStore;
use crate::models::{
    AnnotationInfo, CallGraphPath, CalleeInfo, CallerInfo, ClassInfo, FieldInfo, FunctionInfo,
    GraphDirection, ImportInfo, SymbolRefInfo,
};

pub(crate) struct DbProjectAnalyzer {
    conn: Connection,
    language: Option<String>,
}

impl DbProjectAnalyzer {
    pub(crate) fn from_current_dir_if_compatible(
        root_path: &str,
        language: Option<&str>,
    ) -> anyhow::Result<Option<Self>> {
        let db_path = std::env::current_dir()?.join("tsa.db");
        if !db_path.exists() {
            return Ok(None);
        }
        let store = match IndexStore::open_if_compatible(&db_path, root_path, language)? {
            Some(store) => store,
            None => return Ok(None),
        };
        Ok(Some(Self {
            conn: store.into_connection(),
            language: language.map(str::to_string),
        }))
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

    pub(crate) fn find_symbols(&self, name: &str) -> anyhow::Result<Vec<SymbolRefInfo>> {
        LookupQuery::new(self.query_context()).find_symbol_refs(name)
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

    pub(crate) fn find_super_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        ClassHierarchyQuery::new(self.query_context()).find_super_classes(class_name)
    }

    pub(crate) fn find_sub_classes(&self, class_name: &str) -> anyhow::Result<Vec<ClassInfo>> {
        ClassHierarchyQuery::new(self.query_context()).find_sub_classes(class_name)
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

    fn query_context(&self) -> QueryContext<'_> {
        QueryContext {
            conn: &self.conn,
            language: self.language.as_deref(),
        }
    }
}
