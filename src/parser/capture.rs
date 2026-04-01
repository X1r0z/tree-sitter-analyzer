use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, QueryCursor};

use super::ParseContext;
use crate::languages::{compiled_query, QueryKind};

pub(crate) struct CallCaptureMatch<'a> {
    pub(crate) call: Node<'a>,
    pub(crate) callee: Option<Node<'a>>,
    pub(crate) method: Option<Node<'a>>,
    pub(crate) object: Option<Node<'a>>,
}

pub(crate) struct ImportCaptureMatch<'a> {
    pub(crate) import: Option<Node<'a>>,
    pub(crate) module: Node<'a>,
}

pub(crate) fn collect_capture_pairs<'a>(
    context: &'a ParseContext,
    kind: QueryKind,
    first_capture: &str,
    second_capture: &str,
) -> Vec<(Node<'a>, Node<'a>)> {
    compiled_query(context.language(), kind).map(|query| {
        let capture_names = query.capture_names();
        let Some(first_index) = capture_names
            .iter()
            .position(|name| *name == first_capture)
            .map(|idx| idx as u32)
        else {
            return Vec::new();
        };
        let Some(second_index) = capture_names
            .iter()
            .position(|name| *name == second_capture)
            .map(|idx| idx as u32)
        else {
            return Vec::new();
        };

        let mut cursor = QueryCursor::new();
        let mut capture_matches =
            cursor.matches(query, context.tree().root_node(), context.source());
        let mut pairs = Vec::new();
        while let Some(capture_match) = capture_matches.next() {
            let mut first = None;
            let mut second = None;
            for capture in capture_match.captures {
                if capture.index == first_index {
                    first = Some(capture.node);
                } else if capture.index == second_index {
                    second = Some(capture.node);
                }
            }
            if let (Some(first), Some(second)) = (first, second) {
                pairs.push((first, second));
            }
        }
        pairs
    })
    .unwrap_or_default()
}

pub(crate) fn collect_call_capture_matches<'a>(
    context: &'a ParseContext,
    kind: QueryKind,
) -> Vec<CallCaptureMatch<'a>> {
    compiled_query(context.language(), kind).map(|query| {
        let capture_names = query.capture_names();
        let call_index = capture_names
            .iter()
            .position(|name| *name == "call")
            .map(|idx| idx as u32);
        let callee_index = capture_names
            .iter()
            .position(|name| *name == "callee")
            .map(|idx| idx as u32);
        let method_index = capture_names
            .iter()
            .position(|name| *name == "method")
            .map(|idx| idx as u32);
        let object_index = capture_names
            .iter()
            .position(|name| *name == "object")
            .map(|idx| idx as u32);
        let Some(call_index) = call_index else {
            return Vec::new();
        };

        let mut cursor = QueryCursor::new();
        let mut capture_matches =
            cursor.matches(query, context.tree().root_node(), context.source());
        let mut out = Vec::new();
        while let Some(capture_match) = capture_matches.next() {
            let mut call = None;
            let mut callee = None;
            let mut method = None;
            let mut object = None;
            for capture in capture_match.captures {
                if capture.index == call_index {
                    call = Some(capture.node);
                } else if callee_index == Some(capture.index) {
                    callee = Some(capture.node);
                } else if method_index == Some(capture.index) {
                    method = Some(capture.node);
                } else if object_index == Some(capture.index) {
                    object = Some(capture.node);
                }
            }
            if let Some(call) = call {
                out.push(CallCaptureMatch {
                    call,
                    callee,
                    method,
                    object,
                });
            }
        }
        out
    })
    .unwrap_or_default()
}

pub(crate) fn collect_import_capture_matches<'a>(
    context: &'a ParseContext,
    kind: QueryKind,
) -> Vec<ImportCaptureMatch<'a>> {
    compiled_query(context.language(), kind).map(|query| {
        let capture_names = query.capture_names();
        let import_index = capture_names
            .iter()
            .position(|name| *name == "import")
            .map(|idx| idx as u32);
        let module_index = capture_names
            .iter()
            .position(|name| *name == "module")
            .map(|idx| idx as u32);
        let Some(module_index) = module_index else {
            return Vec::new();
        };

        let mut cursor = QueryCursor::new();
        let mut capture_matches =
            cursor.matches(query, context.tree().root_node(), context.source());
        let mut out = Vec::new();
        while let Some(capture_match) = capture_matches.next() {
            let mut import = None;
            let mut module = None;
            for capture in capture_match.captures {
                if import_index == Some(capture.index) {
                    import = Some(capture.node);
                } else if capture.index == module_index {
                    module = Some(capture.node);
                }
            }
            if let Some(module) = module {
                out.push(ImportCaptureMatch { import, module });
            }
        }
        out
    })
    .unwrap_or_default()
}
