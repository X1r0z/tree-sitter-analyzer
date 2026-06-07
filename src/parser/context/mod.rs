use std::borrow::Cow;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser, Tree};

use crate::languages::{detect_language, LanguageInfo, QueryKind};
use crate::models::Location;

mod caches;
mod enclosing;
mod structural_index;
mod symbols;

use caches::{NodeId, ParseCaches};

pub(crate) use caches::{PythonPropertyCallers, PythonPropertyDefinitions, PythonPropertyIndexes};
pub(crate) use enclosing::EnclosingContext;
pub(crate) use symbols::collect_class_fields;

pub(crate) struct ParseInput {
    pub(crate) file_path: PathBuf,
    pub(crate) language: &'static LanguageInfo,
    pub(crate) source: Vec<u8>,
    pub(crate) tree: Tree,
}

pub(crate) struct ParseContext {
    pub(crate) input: ParseInput,
    pub(crate) caches: RefCell<ParseCaches>,
}

impl ParseContext {
    pub(crate) fn new(file_path: &str) -> anyhow::Result<Self> {
        let source = fs::read(file_path)?;
        Self::from_source(file_path, source)
    }

    pub(crate) fn from_source(file_path: &str, source: Vec<u8>) -> anyhow::Result<Self> {
        let path = Path::new(file_path);
        let language = detect_language(path)
            .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {file_path}"))?;
        let engine = language.engine();
        let mut parser = Parser::new();
        parser.set_language(&engine.ts_language())?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse: {file_path}"))?;

        Ok(Self {
            input: ParseInput {
                file_path: path.to_path_buf(),
                language,
                source,
                tree,
            },
            caches: RefCell::new(ParseCaches::default()),
        })
    }

    pub(crate) fn file_path(&self) -> &Path {
        &self.input.file_path
    }

    pub(crate) fn language(&self) -> &str {
        self.engine().id()
    }

    pub(crate) fn source(&self) -> &[u8] {
        &self.input.source
    }

    pub(crate) fn tree(&self) -> &Tree {
        &self.input.tree
    }

    pub(crate) fn engine(&self) -> &'static dyn crate::languages::LanguageEngine {
        self.input.language.engine()
    }

    pub(crate) fn query(&self, kind: QueryKind) -> &'static tree_sitter::Query {
        self.input.language.query(kind)
    }

    pub(crate) fn resolve_call_targets(
        &self,
        function_node: Node<'_>,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        self.engine()
            .resolve_call_targets(self, function_node, call_node, identifier_name)
    }

    pub(crate) fn node_id(node: Node<'_>) -> NodeId {
        NodeId::from(node)
    }

    pub(crate) fn node_bytes<'a>(&'a self, node: Node) -> &'a [u8] {
        &self.input.source[node.start_byte()..node.end_byte()]
    }

    pub(crate) fn node_text(&self, node: Node) -> String {
        self.node_text_lossy(node).into_owned()
    }

    pub(crate) fn node_text_lossy<'a>(&'a self, node: Node) -> Cow<'a, str> {
        String::from_utf8_lossy(self.node_bytes(node))
    }

    pub(crate) fn node_text_unquoted(&self, node: Node) -> Cow<'_, str> {
        let bytes = self.node_bytes(node);
        if bytes.len() >= 2 {
            let first = bytes[0];
            let last = bytes[bytes.len() - 1];
            if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                return String::from_utf8_lossy(&bytes[1..bytes.len() - 1]);
            }
        }
        self.node_text_lossy(node)
    }

    pub(crate) fn node_text_eq(&self, node: Node, text: &str) -> bool {
        self.node_bytes(node) == text.as_bytes()
    }

    pub(crate) fn node_text_trimmed_eq(&self, node: Node, text: &str) -> bool {
        self.node_text_lossy(node).trim() == text
    }

    pub(crate) fn node_location(&self, node: Node) -> Location {
        Location {
            file: self.file_path().to_string_lossy().into_owned(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        }
    }

    pub(crate) fn source_slice(&self, start_byte: usize, end_byte: usize) -> String {
        String::from_utf8_lossy(&self.input.source[start_byte..end_byte]).into_owned()
    }
}
