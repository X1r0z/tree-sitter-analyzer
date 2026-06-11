use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, QueryMatch};

use super::ParseContext;
use crate::languages::QueryKind;

#[derive(Clone, Copy)]
pub(crate) struct CallCaptureMatch<'a> {
    pub(crate) call: Node<'a>,
    pub(crate) callee: Option<Node<'a>>,
    pub(crate) method: Option<Node<'a>>,
    pub(crate) object: Option<Node<'a>>,
}

#[derive(Clone, Copy)]
pub(crate) struct CallCaptureIndices {
    call: u32,
    callee: Option<u32>,
    method: Option<u32>,
    object: Option<u32>,
}

impl CallCaptureIndices {
    pub(crate) fn for_query(query: &Query) -> Option<Self> {
        Some(Self {
            call: query.capture_index_for_name("call")?,
            callee: query.capture_index_for_name("callee"),
            method: query.capture_index_for_name("method"),
            object: query.capture_index_for_name("object"),
        })
    }
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
    let query = context.query(kind);
    let capture_names = query.capture_names();
    let Some(first_index) = capture_names
        .iter()
        .position(|name| *name == first_capture)
        .and_then(|idx| u32::try_from(idx).ok())
    else {
        return Vec::new();
    };
    let Some(second_index) = capture_names
        .iter()
        .position(|name| *name == second_capture)
        .and_then(|idx| u32::try_from(idx).ok())
    else {
        return Vec::new();
    };

    let mut cursor = QueryCursor::new();
    let mut capture_matches = cursor.matches(query, context.tree().root_node(), context.source());
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
}

pub(crate) fn collect_call_matches(context: &ParseContext) -> Vec<CallCaptureMatch<'_>> {
    let query = context.query(QueryKind::Call);
    let Some(indices) = CallCaptureIndices::for_query(query) else {
        return Vec::new();
    };

    let mut cursor = QueryCursor::new();
    let mut capture_matches = cursor.matches(query, context.tree().root_node(), context.source());
    let mut out = Vec::new();
    while let Some(capture_match) = capture_matches.next() {
        if let Some(matched) = decode_call_match(capture_match, indices) {
            out.push(matched);
        }
    }
    out
}

pub(crate) fn decode_call_match<'a>(
    capture_match: &QueryMatch<'_, 'a>,
    indices: CallCaptureIndices,
) -> Option<CallCaptureMatch<'a>> {
    let mut call = None;
    let mut callee = None;
    let mut method = None;
    let mut object = None;

    for capture in capture_match.captures {
        if capture.index == indices.call {
            call = Some(capture.node);
        } else if indices.callee == Some(capture.index) {
            callee = Some(capture.node);
        } else if indices.method == Some(capture.index) {
            method = Some(capture.node);
        } else if indices.object == Some(capture.index) {
            object = Some(capture.node);
        }
    }

    Some(CallCaptureMatch {
        call: call?,
        callee,
        method,
        object,
    })
}

pub(crate) fn collect_import_matches(
    context: &ParseContext,
    kind: QueryKind,
) -> Vec<ImportCaptureMatch<'_>> {
    let query = context.query(kind);
    let capture_names = query.capture_names();
    let import_index = capture_names
        .iter()
        .position(|name| *name == "import")
        .and_then(|idx| u32::try_from(idx).ok());
    let module_index = capture_names
        .iter()
        .position(|name| *name == "module")
        .and_then(|idx| u32::try_from(idx).ok());
    let Some(module_index) = module_index else {
        return Vec::new();
    };

    let mut cursor = QueryCursor::new();
    let mut capture_matches = cursor.matches(query, context.tree().root_node(), context.source());
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
}
