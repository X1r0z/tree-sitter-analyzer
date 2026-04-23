use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use tree_sitter::{Node, Parser, Tree};

use super::capture;
use super::languages::python;
use crate::languages::{detect_language_engine, LanguageEngine, QueryKind};
use crate::models::{
    AnnotationInfo, CallInfo, ClassInfo, FieldInfo, FunctionInfo, FunctionParamInfo, ImportInfo,
    Location, PythonPropertyCallerInfo, PythonPropertyInfo, RefInfo, RefKey,
};

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

pub(crate) struct ParseInput {
    pub(crate) file_path: PathBuf,
    pub(crate) engine: &'static dyn LanguageEngine,
    pub(crate) language: String,
    pub(crate) source: Vec<u8>,
    pub(crate) tree: Tree,
}

#[derive(Default)]
pub(crate) struct JsParseCaches {
    pub(crate) alias_resolvers_by_function:
        HashMap<NodeId, super::languages::javascript::JsAliasResolver>,
    pub(crate) receiver_resolvers_by_function:
        HashMap<NodeId, super::languages::javascript::JsReceiverResolver>,
    pub(crate) semantic_facts: Option<Rc<super::languages::javascript::JsSemanticFacts>>,
    pub(crate) type_index: Option<Rc<super::languages::javascript::JsTypeIndex>>,
    pub(crate) receiver_index: Option<Rc<super::languages::javascript::JsReceiverIndex>>,
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
    structural_index: Option<StructuralIndexData>,
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

pub(crate) struct EnclosingContext<'a> {
    pub(crate) function_name: Option<String>,
    pub(crate) class_name: Option<String>,
    pub(crate) function_node: Option<Node<'a>>,
}

#[derive(Clone, Default)]
pub(crate) struct CachedEnclosingNames {
    pub(crate) function_name: Option<String>,
    pub(crate) class_name: Option<String>,
}

pub(crate) struct ParseContext {
    pub(crate) input: ParseInput,
    pub(crate) caches: RefCell<ParseCaches>,
}

#[derive(Clone)]
pub(crate) struct ClassSnapshotData {
    pub(crate) classes: Vec<ClassInfo>,
    pub(crate) fields: Vec<FieldInfo>,
    pub(crate) field_infos_by_class: HashMap<String, Vec<FieldInfo>>,
}

#[derive(Clone)]
struct StructuralIndexData {
    functions_without_bodies: Vec<FunctionInfo>,
    class_snapshot: ClassSnapshotData,
    enclosing_interval_index: EnclosingIntervalIndex,
}

struct FunctionSnapshotEntry {
    start_byte: usize,
    end_byte: usize,
    function: FunctionInfo,
}

struct ClassSnapshotEntry {
    start_byte: usize,
    end_byte: usize,
    class: ClassInfo,
}

#[derive(Clone)]
struct EnclosingIntervalIndex {
    function_ranges: Vec<FunctionRange>,
    class_ranges: Vec<ClassRange>,
}

#[derive(Clone)]
struct FunctionRange {
    start_byte: usize,
    end_byte: usize,
    function_name: String,
}

#[derive(Clone)]
struct ClassRange {
    start_byte: usize,
    end_byte: usize,
    class_name: String,
}

struct EnclosingIntervalLookup {
    function_ranges: Vec<FunctionRange>,
    class_ranges: Vec<ClassRange>,
    next_function_idx: usize,
    next_class_idx: usize,
    function_stack: Vec<FunctionRange>,
    class_stack: Vec<ClassRange>,
}

impl EnclosingIntervalLookup {
    fn new(index: EnclosingIntervalIndex) -> Self {
        Self {
            function_ranges: index.function_ranges,
            class_ranges: index.class_ranges,
            next_function_idx: 0,
            next_class_idx: 0,
            function_stack: Vec::new(),
            class_stack: Vec::new(),
        }
    }

    fn lookup(&mut self, node: Node<'_>) -> CachedEnclosingNames {
        let start = node.start_byte();
        let end = node.end_byte();

        while self.next_function_idx < self.function_ranges.len()
            && self.function_ranges[self.next_function_idx].start_byte <= start
        {
            self.function_stack
                .push(self.function_ranges[self.next_function_idx].clone());
            self.next_function_idx += 1;
        }
        while self
            .function_stack
            .last()
            .is_some_and(|range| range.end_byte < end)
        {
            self.function_stack.pop();
        }

        while self.next_class_idx < self.class_ranges.len()
            && self.class_ranges[self.next_class_idx].start_byte <= start
        {
            self.class_stack
                .push(self.class_ranges[self.next_class_idx].clone());
            self.next_class_idx += 1;
        }
        while self
            .class_stack
            .last()
            .is_some_and(|range| range.end_byte < end)
        {
            self.class_stack.pop();
        }

        let function_name = self
            .function_stack
            .last()
            .and_then(|range| (range.start_byte <= start && end <= range.end_byte).then_some(range))
            .map(|range| range.function_name.clone());

        let class_name = self
            .class_stack
            .last()
            .and_then(|range| (range.start_byte <= start && end <= range.end_byte).then_some(range))
            .map(|range| range.class_name.clone());

        CachedEnclosingNames {
            function_name,
            class_name,
        }
    }
}

impl ParseContext {
    pub(crate) fn new(file_path: &str) -> anyhow::Result<Self> {
        let source = fs::read(file_path)?;
        Self::from_source(file_path, source)
    }

    pub(crate) fn from_source(file_path: &str, source: Vec<u8>) -> anyhow::Result<Self> {
        let path = Path::new(file_path);
        let engine = detect_language_engine(path)
            .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file_path))?;
        let mut parser = Parser::new();
        parser.set_language(&engine.ts_language())?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse: {}", file_path))?;

        Ok(Self {
            input: ParseInput {
                file_path: path.to_path_buf(),
                engine,
                language: engine.id().to_string(),
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
        &self.input.language
    }

    pub(crate) fn source(&self) -> &[u8] {
        &self.input.source
    }

    pub(crate) fn tree(&self) -> &Tree {
        &self.input.tree
    }

    pub(crate) fn engine(&self) -> &'static dyn crate::languages::LanguageEngine {
        self.input.engine
    }

    pub(crate) fn resolve_call_targets_with_function_node(
        &self,
        function_node: Node<'_>,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        self.engine()
            .resolve_call_targets(self, function_node, call_node, identifier_name)
    }

    pub(crate) fn node_id(&self, node: Node<'_>) -> NodeId {
        NodeId::from(node)
    }

    pub(crate) fn node_text(&self, node: Node) -> String {
        self.node_text_lossy(node).into_owned()
    }

    pub(crate) fn node_bytes<'a>(&'a self, node: Node) -> &'a [u8] {
        &self.input.source[node.start_byte()..node.end_byte()]
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
            file: self.file_path().to_string_lossy().into_owned(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        }
    }

    pub(crate) fn source_text(&self, start_byte: usize, end_byte: usize) -> String {
        String::from_utf8_lossy(&self.input.source[start_byte..end_byte]).into_owned()
    }

    pub(crate) fn find_enclosing_context<'a>(&self, node: Node<'a>) -> EnclosingContext<'a> {
        if let Some(class_name) = self.engine().enclosing_class_name(self, node) {
            return EnclosingContext {
                function_name: self.cached_function_name(node),
                class_name: Some(class_name),
                function_node: Some(node),
            };
        }

        let enclosing = self.cached_enclosing_names(node);

        EnclosingContext {
            function_name: enclosing.function_name,
            class_name: enclosing.class_name,
            function_node: self.find_enclosing_function_node(node),
        }
    }

    pub(crate) fn cached_enclosing_names(&self, node: Node<'_>) -> CachedEnclosingNames {
        let node_id = self.node_id(node);
        if let Some(names) = self
            .caches
            .borrow()
            .common
            .enclosing_names_by_node
            .get(&node_id)
        {
            return names.clone();
        }

        let names = self.compute_enclosing_names_by_walk(node);
        self.caches
            .borrow_mut()
            .common
            .enclosing_names_by_node
            .insert(node_id, names.clone());
        names
    }

    pub(crate) fn cached_function_name(&self, node: Node) -> Option<String> {
        let node_id = self.node_id(node);
        if let Some(name) = self
            .caches
            .borrow()
            .common
            .function_names_by_node
            .get(&node_id)
        {
            return name.clone();
        }
        let name = self.engine().function_name(self, node);
        self.caches
            .borrow_mut()
            .common
            .function_names_by_node
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn cached_class_name(&self, node: Node) -> Option<String> {
        let node_id = self.node_id(node);
        if let Some(name) = self
            .caches
            .borrow()
            .common
            .class_names_by_node
            .get(&node_id)
        {
            return name.clone();
        }
        let name = class_name_from_node(self, node);
        self.caches
            .borrow_mut()
            .common
            .class_names_by_node
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn collect_functions(&self, include_body: bool) -> Vec<FunctionInfo> {
        if !include_body {
            self.ensure_structural_index();
            return self
                .caches
                .borrow()
                .common
                .structural_index
                .as_ref()
                .map(|index| index.functions_without_bodies.clone())
                .unwrap_or_default();
        }
        let mut func_pairs: Vec<(Node<'_>, Node<'_>)> =
            capture::collect_capture_pairs(self, QueryKind::Function, "function", "name");
        func_pairs.sort_by_key(|(f, _)| (f.start_byte(), std::cmp::Reverse(f.end_byte())));

        let mut functions = Vec::new();
        let mut seen = HashSet::new();

        for (func_node, name_node) in func_pairs {
            let function_node = self.engine().normalize_function_node(self, func_node);
            let name = self
                .cached_function_name(function_node)
                .unwrap_or_else(|| self.node_text(name_node));
            if name.is_empty() {
                continue;
            }
            let class_name = self.find_enclosing_context(function_node).class_name;
            let key = (
                function_node.start_byte(),
                function_node.end_byte(),
                name.clone(),
                class_name.clone(),
            );
            if !seen.insert(key) {
                continue;
            }
            functions.push(FunctionInfo {
                name,
                location: self.node_location(function_node),
                body: if include_body {
                    self.node_text(function_node)
                } else {
                    String::new()
                },
                class_name,
                params: self.collect_function_params(function_node),
            });
        }

        functions
    }

    pub(crate) fn collect_python_properties(&self) -> Vec<PythonPropertyInfo> {
        python::PythonPropertyAnalyzer::new(self).collect_infos()
    }

    pub(crate) fn collect_python_property_callers(
        &self,
        property_name: Option<&str>,
    ) -> Vec<PythonPropertyCallerInfo> {
        python::PythonPropertyAnalyzer::new(self).collect_callers(property_name)
    }

    pub(crate) fn collect_function_params(&self, function_node: Node) -> Vec<FunctionParamInfo> {
        let target = self.engine().normalize_function_node(self, function_node);
        self.engine().function_params(self, target)
    }

    pub(crate) fn collect_classes(&self) -> Vec<ClassInfo> {
        self.ensure_structural_index();
        self.caches
            .borrow()
            .common
            .structural_index
            .as_ref()
            .map(|index| index.class_snapshot.classes.clone())
            .unwrap_or_default()
    }

    pub(crate) fn collect_field_infos_for_class(&self, class_name: &str) -> Vec<FieldInfo> {
        self.ensure_structural_index();
        self.caches
            .borrow()
            .common
            .structural_index
            .as_ref()
            .and_then(|index| index.class_snapshot.field_infos_by_class.get(class_name))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn collect_class_snapshot(&self) -> ClassSnapshotData {
        self.ensure_structural_index();
        self.caches
            .borrow()
            .common
            .structural_index
            .as_ref()
            .map(|index| index.class_snapshot.clone())
            .unwrap_or_else(|| ClassSnapshotData {
                classes: Vec::new(),
                fields: Vec::new(),
                field_infos_by_class: HashMap::new(),
            })
    }

    pub(crate) fn collect_calls(&self) -> Vec<CallInfo> {
        let mut matches = capture::collect_call_capture_matches(self);
        matches.sort_by_key(|matched| {
            (
                matched.call.start_byte(),
                std::cmp::Reverse(matched.call.end_byte()),
            )
        });
        let is_js_family = matches!(self.language(), "javascript" | "typescript" | "tsx");
        let mut interval_lookup = if is_js_family {
            None
        } else {
            self.ensure_structural_index();
            let index = self
                .caches
                .borrow()
                .common
                .structural_index
                .as_ref()
                .map(|index| index.enclosing_interval_index.clone());
            index.map(EnclosingIntervalLookup::new)
        };

        let mut calls = Vec::new();
        let mut last_call_start = None;
        for matched in matches {
            let call_node = matched.call;
            debug_assert!(
                last_call_start.is_none_or(|previous| previous <= call_node.start_byte()),
                "call query matches should be sorted by start byte"
            );
            last_call_start = Some(call_node.start_byte());
            let enclosing = if is_js_family {
                self.find_enclosing_context(call_node)
            } else {
                let enclosing = interval_lookup
                    .as_mut()
                    .map(|lookup| lookup.lookup(call_node))
                    .unwrap_or_default();
                EnclosingContext {
                    function_name: enclosing.function_name,
                    class_name: enclosing.class_name,
                    function_node: None,
                }
            };

            let resolved = self.engine().resolve_call(self, &matched, &enclosing);
            let callee = resolved.callee;
            let is_method = resolved.is_method;
            let object_name = resolved.object_name;
            let callee_function_node = resolved.callee_function_node;

            if callee.is_empty() {
                continue;
            }
            if !self
                .engine()
                .include_call(self, call_node, &callee, object_name.as_deref())
            {
                continue;
            }

            let call_location = self.node_location(call_node);
            let mut used_resolved_calls = false;
            if is_js_family
                && !is_method
                && callee_function_node
                    .map(|node| node.kind() == "identifier")
                    .unwrap_or(false)
            {
                let Some(function_node) = enclosing.function_node else {
                    continue;
                };
                let resolved =
                    self.resolve_call_targets_with_function_node(function_node, call_node, &callee);
                if !resolved.is_empty() {
                    for resolved_callee in resolved {
                        calls.push(CallInfo {
                            callee: resolved_callee,
                            location: call_location.clone(),
                            caller: enclosing.function_name.clone(),
                            caller_class_name: enclosing.class_name.clone(),
                            object_name: object_name.clone(),
                        });
                    }
                    used_resolved_calls = true;
                }
            }

            if !used_resolved_calls {
                calls.push(CallInfo {
                    callee,
                    location: call_location,
                    caller: enclosing.function_name.clone(),
                    caller_class_name: enclosing.class_name.clone(),
                    object_name,
                });
            }
        }

        calls
    }

    pub(crate) fn collect_imports(&self) -> Vec<ImportInfo> {
        capture::collect_import_capture_matches(self, QueryKind::Import)
            .into_iter()
            .map(|import_match| ImportInfo {
                module: self.node_text_unquoted(import_match.module).into_owned(),
                location: self.node_location(import_match.import.unwrap_or(import_match.module)),
            })
            .collect()
    }

    pub(crate) fn collect_annotations(&self) -> Vec<AnnotationInfo> {
        self.engine().annotations(self)
    }

    pub(crate) fn collect_refs(&self) -> Vec<RefInfo> {
        self.scan_refs(|node| {
            let name = self.node_text(node);
            (!name.is_empty()).then(|| RefInfo {
                name,
                node_type: node.kind().to_string(),
                location: self.node_location(node),
                start_column: node.start_position().column,
                end_column: node.end_position().column,
                context: String::new(),
            })
        })
    }

    pub(crate) fn hydrate_refs(&self, candidates: &[RefInfo]) -> Vec<RefInfo> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let expected: std::collections::HashSet<RefKey> =
            candidates.iter().map(RefKey::from).collect();
        self.scan_refs(|node| {
            let reference = RefInfo {
                name: self.node_text(node),
                node_type: node.kind().to_string(),
                location: self.node_location(node),
                start_column: node.start_position().column,
                end_column: node.end_position().column,
                context: String::new(),
            };
            expected
                .contains(&RefKey::from(&reference))
                .then(|| RefInfo {
                    context: node
                        .parent()
                        .map(|parent| self.node_text(parent))
                        .unwrap_or_default(),
                    ..reference
                })
        })
    }

    fn scan_refs(&self, mut map_node: impl FnMut(Node<'_>) -> Option<RefInfo>) -> Vec<RefInfo> {
        let mut refs = Vec::new();
        let mut stack = vec![self.tree().root_node()];
        while let Some(node) = stack.pop() {
            if node.is_named() {
                let is_ref = self.engine().is_ref_node(node);
                if is_ref {
                    if !self.engine().ref_filter(self, node) {
                        continue;
                    }
                    if let Some(reference) = map_node(node) {
                        refs.push(reference);
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

    fn ensure_structural_index(&self) {
        if self.caches.borrow().common.structural_index.is_some() {
            return;
        }
        let mut func_pairs: Vec<(Node<'_>, Node<'_>)> =
            capture::collect_capture_pairs(self, QueryKind::Function, "function", "name");
        func_pairs.sort_by_key(|(f, _)| (f.start_byte(), std::cmp::Reverse(f.end_byte())));

        let mut function_entries = Vec::new();
        let mut seen = HashSet::new();

        for (func_node, name_node) in func_pairs {
            let function_node = self.engine().normalize_function_node(self, func_node);
            let name = self
                .cached_function_name(function_node)
                .unwrap_or_else(|| self.node_text(name_node));
            if name.is_empty() {
                continue;
            }
            let class_name = self
                .compute_enclosing_names_by_walk(function_node)
                .class_name;
            let key = (
                function_node.start_byte(),
                function_node.end_byte(),
                name.clone(),
                class_name.clone(),
            );
            if !seen.insert(key) {
                continue;
            }
            function_entries.push(FunctionSnapshotEntry {
                start_byte: function_node.start_byte(),
                end_byte: function_node.end_byte(),
                function: FunctionInfo {
                    name,
                    location: self.node_location(function_node),
                    body: String::new(),
                    class_name,
                    params: self.collect_function_params(function_node),
                },
            });
        }

        let functions_without_bodies = function_entries
            .iter()
            .map(|entry| entry.function.clone())
            .collect::<Vec<_>>();

        let mut methods_by_class = std::collections::HashMap::new();
        if self.language() == "go" {
            for function in &functions_without_bodies {
                if let Some(class_name) = function.class_name.clone() {
                    methods_by_class
                        .entry(class_name)
                        .or_insert_with(Vec::new)
                        .push(function.name.clone());
                }
            }
            for methods in methods_by_class.values_mut() {
                methods.sort_unstable();
                methods.dedup();
            }
        }

        let mut class_pairs: Vec<(Node<'_>, Node<'_>)> =
            capture::collect_capture_pairs(self, QueryKind::Class, "class", "name");
        class_pairs.sort_by_key(|(class_node, _)| {
            (
                class_node.start_byte(),
                std::cmp::Reverse(class_node.end_byte()),
            )
        });

        let mut class_entries = Vec::new();
        let mut active_ranges: Vec<usize> = Vec::new();
        let mut seen = HashSet::new();
        let allow_nested_classes = matches!(self.language(), "python" | "java");
        let mut chosen_fields_by_class: HashMap<String, (usize, usize, Vec<FieldInfo>)> =
            HashMap::new();

        for (class_node, name_node) in class_pairs {
            let class_node = self.engine().normalize_class_node(self, class_node);
            let name = self
                .cached_class_name(class_node)
                .unwrap_or_else(|| self.node_text(name_node));
            if name.is_empty() {
                continue;
            }
            let start = class_node.start_byte();
            let end = class_node.end_byte();
            if !seen.insert((start, end, name.clone())) {
                continue;
            }
            while let Some(&active_end) = active_ranges.last() {
                if start >= active_end {
                    active_ranges.pop();
                } else {
                    break;
                }
            }
            let is_nested = !active_ranges.is_empty();
            if is_nested && !allow_nested_classes {
                continue;
            }
            active_ranges.push(end);

            let field_infos = self.engine().class_fields(self, class_node, &name);
            let field_names = field_infos.iter().map(|field| field.name.clone()).collect();
            let class_size = end - start;
            match chosen_fields_by_class.get(&name) {
                Some((best_size, best_start, _))
                    if (*best_size, *best_start) <= (class_size, start) => {}
                _ => {
                    chosen_fields_by_class
                        .insert(name.clone(), (class_size, start, field_infos.clone()));
                }
            }

            let mut method_names = class_method_names(self, class_node);
            if self.language() == "go" {
                if let Some(go_methods) = methods_by_class.get(&name) {
                    method_names.extend(go_methods.iter().cloned());
                    method_names.sort_unstable();
                    method_names.dedup();
                }
            }
            let super_class_names = self.engine().super_types(self, class_node);

            class_entries.push(ClassSnapshotEntry {
                start_byte: start,
                end_byte: end,
                class: ClassInfo {
                    name,
                    kind: class_kind(self, class_node),
                    location: self.node_location(class_node),
                    methods: method_names,
                    fields: field_names,
                    super_classes: super_class_names,
                },
            });
        }

        let classes = class_entries
            .iter()
            .map(|entry| entry.class.clone())
            .collect::<Vec<_>>();
        let field_infos_by_class = chosen_fields_by_class
            .into_iter()
            .map(|(name, (_, _, fields))| (name, fields))
            .collect::<HashMap<_, _>>();
        let fields = classes
            .iter()
            .flat_map(|class| {
                field_infos_by_class
                    .get(&class.name)
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();

        let class_snapshot = ClassSnapshotData {
            classes,
            fields,
            field_infos_by_class,
        };

        let function_ranges = function_entries
            .into_iter()
            .map(|entry| FunctionRange {
                start_byte: entry.start_byte,
                end_byte: entry.end_byte,
                function_name: entry.function.name,
            })
            .collect();
        let class_ranges = class_entries
            .iter()
            .map(|entry| ClassRange {
                start_byte: entry.start_byte,
                end_byte: entry.end_byte,
                class_name: entry.class.name.clone(),
            })
            .collect();

        let enclosing_interval_index = EnclosingIntervalIndex {
            function_ranges,
            class_ranges,
        };

        let structural_index = StructuralIndexData {
            functions_without_bodies,
            class_snapshot,
            enclosing_interval_index,
        };
        self.caches.borrow_mut().common.structural_index = Some(structural_index);
    }

    fn find_enclosing_function_node<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        let mut current = Some(node);
        while let Some(current_node) = current {
            if is_function_like(current_node.kind()) {
                return Some(current_node);
            }
            current = current_node.parent();
        }
        None
    }

    fn compute_enclosing_names_by_walk(&self, node: Node<'_>) -> CachedEnclosingNames {
        let mut current = Some(node);
        let mut function_name: Option<String> = None;
        let mut class_name: Option<String> = None;

        while let Some(current_node) = current {
            if function_name.is_none() && is_function_like(current_node.kind()) {
                function_name = self.cached_function_name(current_node);
            }

            if class_name.is_none() {
                class_name = self.engine().enclosing_class_name(self, current_node);
            }

            if class_name.is_none()
                && matches!(
                    current_node.kind(),
                    "class_definition"
                        | "class_declaration"
                        | "class_body"
                        | "interface_declaration"
                        | "enum_declaration"
                        | "record_declaration"
                        | "annotation_type_declaration"
                )
            {
                class_name = self.cached_class_name(current_node);
            }

            if function_name.is_some() && class_name.is_some() {
                break;
            }
            current = current_node.parent();
        }

        CachedEnclosingNames {
            function_name,
            class_name,
        }
    }
}

fn is_function_like(node_kind: &str) -> bool {
    matches!(
        node_kind,
        "function_definition"
            | "async_function_definition"
            | "function_declaration"
            | "method_definition"
            | "arrow_function"
            | "method_declaration"
            | "constructor_declaration"
            | "function_expression"
            | "func_literal"
    )
}

fn class_name_from_node(context: &ParseContext, node: Node<'_>) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let class_name = context.node_text(name_node);
        if !class_name.is_empty() {
            return Some(class_name);
        }
    }
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if matches!(child.kind(), "identifier" | "type_identifier" | "name") {
            let class_name = context.node_text(child);
            if !class_name.is_empty() {
                return Some(class_name);
            }
        }
    }
    None
}

fn class_kind(context: &ParseContext, node: Node<'_>) -> String {
    match context.language() {
        "python" | "javascript" => "class".to_string(),
        "typescript" | "tsx" => match node.kind() {
            "abstract_class_declaration" => "abstract_class".to_string(),
            "interface_declaration" => "interface".to_string(),
            "type_alias_declaration" => "alias".to_string(),
            "enum_declaration" => "enum".to_string(),
            _ => "class".to_string(),
        },
        "java" => match node.kind() {
            "interface_declaration" => "interface".to_string(),
            "enum_declaration" => "enum".to_string(),
            "record_declaration" => "record".to_string(),
            "annotation_type_declaration" => "annotation".to_string(),
            _ => "class".to_string(),
        },
        "go" => node
            .children(&mut node.walk())
            .find(|child| child.kind() == "type_spec")
            .and_then(|type_spec| type_spec.child_by_field_name("type"))
            .map(|type_node| match type_node.kind() {
                "interface_type" => "interface",
                "struct_type" => "struct",
                _ => "struct",
            })
            .unwrap_or("struct")
            .to_string(),
        _ => "class".to_string(),
    }
}

pub(crate) fn collect_field_infos_from_declarations(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
    include_embedded_type_names: bool,
) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![class_node];

    while let Some(node) = stack.pop() {
        if node.id() != class_node.id()
            && matches!(
                node.kind(),
                "class_definition"
                    | "class_declaration"
                    | "class"
                    | "interface_declaration"
                    | "enum_declaration"
                    | "record_declaration"
                    | "annotation_type_declaration"
            )
        {
            let nested_name = node
                .child_by_field_name("name")
                .map(|child| parser.node_text(child))
                .unwrap_or_default();
            if !nested_name.is_empty() && nested_name != class_name {
                continue;
            }
        }

        if matches!(
            node.kind(),
            "function_definition"
                | "method_definition"
                | "method_declaration"
                | "constructor_declaration"
        ) {
            continue;
        }

        if matches!(node.kind(), "field_definition" | "field_declaration") {
            let mut names = Vec::new();
            let mut field_type = node
                .child_by_field_name("type")
                .map(|child| parser.node_text(child));

            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier"
                ) {
                    names.push(parser.node_text(child));
                } else if child.kind() == "variable_declarator" {
                    for j in 0..child.child_count() {
                        let sub = child.child(j as u32).unwrap();
                        if sub.kind() == "identifier" {
                            names.push(parser.node_text(sub));
                            break;
                        }
                    }
                } else if field_type.is_none()
                    && matches!(
                        child.kind(),
                        "type_annotation"
                            | "type"
                            | "type_identifier"
                            | "integral_type"
                            | "floating_point_type"
                            | "boolean_type"
                            | "generic_type"
                            | "array_type"
                            | "scoped_type_identifier"
                    )
                {
                    field_type = Some(parser.node_text(child));
                }
            }

            if include_embedded_type_names && names.is_empty() {
                if let Some(field_type_text) = field_type.as_ref() {
                    let type_str = field_type_text.trim_start_matches('*');
                    let embedded_type_name = if type_str.contains('.') {
                        type_str.rsplit('.').next().unwrap_or(type_str)
                    } else {
                        type_str
                    };
                    names.push(embedded_type_name.to_string());
                }
            }

            for name in names {
                if !name.is_empty() && seen.insert(name.clone()) {
                    fields.push(FieldInfo {
                        name,
                        location: parser.node_location(node),
                        field_type: field_type.clone(),
                        class_name: Some(class_name.to_string()),
                    });
                }
            }
            continue;
        }

        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i as u32) {
                stack.push(child);
            }
        }
    }

    fields
}

fn class_method_names(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
    let mut methods = Vec::new();
    let mut stack = vec![class_node];
    while let Some(node) = stack.pop() {
        if node.id() != class_node.id() && is_nested_class_boundary(node.kind()) {
            continue;
        }
        if matches!(
            node.kind(),
            "function_definition"
                | "method_definition"
                | "method_declaration"
                | "constructor_declaration"
                | "method_elem"
                | "method_spec"
        ) {
            for i in 0..node.child_count() {
                let child = node.child(i as u32).unwrap();
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier" | "name"
                ) {
                    methods.push(parser.node_text(child));
                    break;
                }
            }
            continue;
        }
        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i as u32) {
                stack.push(child);
            }
        }
    }
    methods
}

fn is_nested_class_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "class_definition"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration"
    )
}
