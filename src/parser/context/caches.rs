use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use tree_sitter::Node;

use crate::models::PythonPropertyCallerInfo;
use crate::parser::languages::javascript;

use super::structural_index::StructuralIndexData;

pub(crate) type PythonPropertyKey = (String, Option<String>);
pub(crate) type PythonPropertyDefinitions = HashSet<PythonPropertyKey>;
pub(crate) type PythonPropertyCallers = HashMap<String, Vec<PythonPropertyCallerInfo>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NodeId(usize);

impl NodeId {
    pub(crate) fn new(value: usize) -> Self {
        Self(value)
    }
}

impl From<usize> for NodeId {
    fn from(value: usize) -> Self {
        Self::new(value)
    }
}

impl<'a> From<Node<'a>> for NodeId {
    fn from(node: Node<'a>) -> Self {
        Self::new(node.id())
    }
}

#[derive(Default)]
pub(crate) struct JsParseCaches {
    pub(crate) alias_resolvers_by_function: HashMap<NodeId, javascript::JsAliasResolver>,
    pub(crate) receiver_resolvers_by_function: HashMap<NodeId, javascript::JsReceiverResolver>,
    pub(crate) semantic_facts: Option<Rc<javascript::JsSemanticFacts>>,
    pub(crate) type_index: Option<Rc<javascript::JsTypeIndex>>,
    pub(crate) receiver_index: Option<Rc<javascript::JsReceiverIndex>>,
    pub(crate) class_names: Option<Rc<HashSet<String>>>,
}

pub(crate) struct PythonPropertyIndexes {
    pub(crate) definitions: PythonPropertyDefinitions,
    pub(crate) callers_by_property: Option<PythonPropertyCallers>,
}

#[derive(Default)]
pub(crate) struct PythonParseCaches {
    pub(crate) property_indexes: Option<PythonPropertyIndexes>,
}

#[derive(Default)]
pub(crate) struct CommonParseCaches {
    pub(crate) function_names_by_node: HashMap<NodeId, Option<String>>,
    pub(crate) class_names_by_node: HashMap<NodeId, Option<String>>,
    pub(crate) enclosing_names_by_node: HashMap<NodeId, CachedEnclosingNames>,
    pub(super) structural_index: Option<StructuralIndexData>,
}

#[derive(Default)]
pub(crate) struct LanguageSpecificCaches {
    pub(crate) js: JsParseCaches,
    pub(crate) python: PythonParseCaches,
}

#[derive(Default)]
pub(crate) struct ParseCaches {
    pub(crate) common: CommonParseCaches,
    pub(crate) language: LanguageSpecificCaches,
}

#[derive(Clone, Default)]
pub(crate) struct CachedEnclosingNames {
    pub(crate) function_name: Option<String>,
    pub(crate) class_name: Option<String>,
}
