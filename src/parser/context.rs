use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use tree_sitter::{Node, Parser, Tree};

use crate::languages::{detect_language, find_language, find_language_info};
use crate::models::{Location, PythonPropertyCallerInfo};

pub(crate) type PythonPropertyKey = (String, Option<String>);
pub(crate) type PythonPropertyDefinitions = HashSet<PythonPropertyKey>;
pub(crate) type PythonPropertyCallers = HashMap<String, Vec<PythonPropertyCallerInfo>>;

pub(crate) struct ParseContext {
    pub(crate) file_path: String,
    pub(crate) language: String,
    pub(crate) source: Vec<u8>,
    pub(crate) tree: Tree,
    pub(crate) function_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    pub(crate) class_names_by_node: RefCell<HashMap<usize, Option<String>>>,
    pub(crate) js_alias_resolvers_by_function:
        RefCell<HashMap<usize, super::languages::javascript::JsAliasResolverState>>,
}

impl ParseContext {
    pub(crate) fn new(file_path: &str) -> anyhow::Result<Self> {
        let path = Path::new(file_path);
        let language = detect_language(path)
            .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file_path))?;
        let ts_lang = find_language(language)
            .ok_or_else(|| anyhow::anyhow!("Unsupported language: {}", language))?;
        find_language_info(language)
            .ok_or_else(|| anyhow::anyhow!("No language info for: {}", language))?;

        let source = fs::read(file_path)?;
        let mut parser = Parser::new();
        parser.set_language(&ts_lang)?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse: {}", file_path))?;

        Ok(Self {
            file_path: file_path.to_string(),
            language: language.to_string(),
            source,
            tree,
            function_names_by_node: RefCell::new(HashMap::new()),
            class_names_by_node: RefCell::new(HashMap::new()),
            js_alias_resolvers_by_function: RefCell::new(HashMap::new()),
        })
    }

    pub(crate) fn node_text(&self, node: Node) -> String {
        self.node_text_lossy(node).into_owned()
    }

    pub(crate) fn node_bytes<'a>(&'a self, node: Node) -> &'a [u8] {
        &self.source[node.start_byte()..node.end_byte()]
    }

    pub(crate) fn node_text_lossy<'a>(&'a self, node: Node) -> Cow<'a, str> {
        String::from_utf8_lossy(self.node_bytes(node))
    }

    pub(crate) fn node_text_eq(&self, node: Node, text: &str) -> bool {
        self.node_bytes(node) == text.as_bytes()
    }

    pub(crate) fn node_trimmed_text_eq(&self, node: Node, text: &str) -> bool {
        self.node_text_lossy(node).trim() == text
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

    pub(crate) fn node_location(&self, node: Node) -> Location {
        Location {
            file: self.file_path.clone(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        }
    }

    pub(crate) fn source_text(&self, start_byte: usize, end_byte: usize) -> String {
        String::from_utf8_lossy(&self.source[start_byte..end_byte]).into_owned()
    }
}
