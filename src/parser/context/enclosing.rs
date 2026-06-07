use tree_sitter::Node;

use super::caches::CachedEnclosingNames;
use super::class_info::class_name;
use super::ParseContext;

pub(crate) struct EnclosingContext<'a> {
    pub(crate) function_name: Option<String>,
    pub(crate) class_name: Option<String>,
    pub(crate) function_node: Option<Node<'a>>,
}

#[derive(Clone)]
pub(super) struct EnclosingIntervalIndex {
    pub(super) function_ranges: Vec<FunctionRange>,
    pub(super) class_ranges: Vec<ClassRange>,
}

#[derive(Clone)]
pub(super) struct FunctionRange {
    pub(super) start_byte: usize,
    pub(super) end_byte: usize,
    pub(super) function_name: String,
}

#[derive(Clone)]
pub(super) struct ClassRange {
    pub(super) start_byte: usize,
    pub(super) end_byte: usize,
    pub(super) class_name: String,
}

pub(super) struct EnclosingIntervalLookup {
    function_ranges: Vec<FunctionRange>,
    class_ranges: Vec<ClassRange>,
    next_function_idx: usize,
    next_class_idx: usize,
    function_stack: Vec<FunctionRange>,
    class_stack: Vec<ClassRange>,
}

impl EnclosingIntervalLookup {
    pub(super) fn new(index: EnclosingIntervalIndex) -> Self {
        Self {
            function_ranges: index.function_ranges,
            class_ranges: index.class_ranges,
            next_function_idx: 0,
            next_class_idx: 0,
            function_stack: Vec::new(),
            class_stack: Vec::new(),
        }
    }

    pub(super) fn enclosing_at(&mut self, node: Node<'_>) -> CachedEnclosingNames {
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
            function_node: Self::find_enclosing_function_node(node),
        }
    }

    pub(crate) fn cached_enclosing_names(&self, node: Node<'_>) -> CachedEnclosingNames {
        let node_id = Self::node_id(node);
        if let Some(names) = self
            .caches
            .borrow()
            .common
            .enclosing_names_by_node
            .get(&node_id)
        {
            return names.clone();
        }

        let names = self.compute_enclosing_names(node);
        self.caches
            .borrow_mut()
            .common
            .enclosing_names_by_node
            .insert(node_id, names.clone());
        names
    }

    pub(crate) fn cached_function_name(&self, node: Node) -> Option<String> {
        let node_id = Self::node_id(node);
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
        let node_id = Self::node_id(node);
        if let Some(name) = self
            .caches
            .borrow()
            .common
            .class_names_by_node
            .get(&node_id)
        {
            return name.clone();
        }
        let name = class_name(self, node);
        self.caches
            .borrow_mut()
            .common
            .class_names_by_node
            .insert(node_id, name.clone());
        name
    }

    pub(super) fn compute_enclosing_names(&self, node: Node<'_>) -> CachedEnclosingNames {
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

    fn find_enclosing_function_node(node: Node<'_>) -> Option<Node<'_>> {
        let mut current = Some(node);
        while let Some(current_node) = current {
            if is_function_like(current_node.kind()) {
                return Some(current_node);
            }
            current = current_node.parent();
        }
        None
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
