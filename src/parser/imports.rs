use super::query;
use super::ParseContext;
use crate::languages::QueryKind;
use crate::models::ImportInfo;

impl ParseContext {
    pub(crate) fn collect_imports(&self) -> Vec<ImportInfo> {
        query::query_capture_nodes(self, QueryKind::Import, "module")
            .into_iter()
            .map(|node| ImportInfo {
                module: self.node_text_unquoted(node).into_owned(),
                location: self.node_location(node),
            })
            .collect()
    }
}
