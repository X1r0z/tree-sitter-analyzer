use tree_sitter::Node;

use crate::models::{SymbolRefInfo, SymbolRefKey};
use crate::parser::BaseParser;

fn is_symbol_ref_node(parser: &BaseParser, node: Node<'_>) -> bool {
    match parser.language.as_str() {
        "python" => matches!(
            node.kind(),
            "identifier" | "dotted_name" | "relative_import"
        ),
        "javascript" | "typescript" | "tsx" => matches!(
            node.kind(),
            "identifier"
                | "property_identifier"
                | "private_property_identifier"
                | "field_identifier"
                | "type_identifier"
        ),
        "java" => matches!(
            node.kind(),
            "identifier" | "type_identifier" | "scoped_identifier" | "scoped_type_identifier"
        ),
        "go" => matches!(
            node.kind(),
            "identifier" | "field_identifier" | "type_identifier" | "qualified_type"
        ),
        _ => false,
    }
}

fn scan_symbols(
    parser: &BaseParser,
    mut map_node: impl FnMut(Node<'_>) -> Option<SymbolRefInfo>,
) -> Vec<SymbolRefInfo> {
    let mut refs = Vec::new();
    let mut stack = vec![parser.tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.is_named() {
            if is_symbol_ref_node(parser, node) {
                if let Some(symbol) = map_node(node) {
                    refs.push(symbol);
                }
            }
            for i in (0..node.named_child_count()).rev() {
                if let Some(child) = node.named_child(i as u32) {
                    stack.push(child);
                }
            }
        }
    }
    refs
}

pub(crate) fn collect_all(parser: &BaseParser) -> Vec<SymbolRefInfo> {
    scan_symbols(parser, |node| {
        let name = parser.node_text(node);
        (!name.is_empty()).then(|| SymbolRefInfo {
            name,
            node_type: node.kind().to_string(),
            location: parser.node_location(node),
            start_column: node.start_position().column,
            end_column: node.end_position().column,
            context: String::new(),
        })
    })
}

pub(crate) fn find(parser: &BaseParser, name: &str, with_context: bool) -> Vec<SymbolRefInfo> {
    let name_bytes = name.as_bytes();
    if name_bytes.is_empty()
        || !parser
            .source
            .windows(name_bytes.len())
            .any(|w| w == name_bytes)
    {
        return Vec::new();
    }

    scan_symbols(parser, |node| {
        (parser.node_bytes(node) == name_bytes).then(|| SymbolRefInfo {
            name: name.to_string(),
            node_type: node.kind().to_string(),
            location: parser.node_location(node),
            start_column: node.start_position().column,
            end_column: node.end_position().column,
            context: if with_context {
                node.parent()
                    .map(|parent| parser.node_text(parent))
                    .unwrap_or_default()
            } else {
                String::new()
            },
        })
    })
}

pub(crate) fn hydrate(parser: &BaseParser, candidates: &[SymbolRefInfo]) -> Vec<SymbolRefInfo> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let expected: std::collections::HashSet<SymbolRefKey> =
        candidates.iter().map(SymbolRefKey::from_symbol).collect();
    scan_symbols(parser, |node| {
        let symbol = SymbolRefInfo {
            name: parser.node_text(node),
            node_type: node.kind().to_string(),
            location: parser.node_location(node),
            start_column: node.start_position().column,
            end_column: node.end_position().column,
            context: String::new(),
        };
        expected
            .contains(&SymbolRefKey::from_symbol(&symbol))
            .then(|| SymbolRefInfo {
                context: node
                    .parent()
                    .map(|parent| parser.node_text(parent))
                    .unwrap_or_default(),
                ..symbol
            })
    })
}
