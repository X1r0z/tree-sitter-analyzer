use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor};

use super::ParseContext;
use crate::languages::{compiled_query, QueryKind};

pub(crate) struct CallCaptureMatch<'a> {
    pub(crate) call: Node<'a>,
    pub(crate) callee: Option<Node<'a>>,
    pub(crate) method: Option<Node<'a>>,
    pub(crate) object: Option<Node<'a>>,
}

fn with_compiled_capture_query<T>(
    context: &ParseContext,
    kind: QueryKind,
    build: impl FnOnce(&Query) -> T,
) -> Option<T> {
    compiled_query(&context.language, kind).map(build)
}

pub(crate) fn collect_capture_nodes<'a>(
    context: &'a ParseContext,
    kind: QueryKind,
    capture_name: &str,
) -> Vec<Node<'a>> {
    with_compiled_capture_query(context, kind, |query| {
        let Some(capture_index) = query
            .capture_names()
            .iter()
            .position(|name| *name == capture_name)
            .map(|idx| idx as u32)
        else {
            return Vec::new();
        };

        let mut cursor = QueryCursor::new();
        let mut capture_matches =
            cursor.matches(query, context.tree.root_node(), context.source.as_slice());
        let mut nodes = Vec::new();
        while let Some(capture_match) = capture_matches.next() {
            for capture in capture_match.captures {
                if capture.index == capture_index {
                    nodes.push(capture.node);
                }
            }
        }
        nodes
    })
    .unwrap_or_default()
}

pub(crate) fn collect_capture_pairs<'a>(
    context: &'a ParseContext,
    kind: QueryKind,
    first_capture: &str,
    second_capture: &str,
) -> Vec<(Node<'a>, Node<'a>)> {
    with_compiled_capture_query(context, kind, |query| {
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
            cursor.matches(query, context.tree.root_node(), context.source.as_slice());
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
    with_compiled_capture_query(context, kind, |query| {
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
            cursor.matches(query, context.tree.root_node(), context.source.as_slice());
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
