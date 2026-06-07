use std::collections::HashSet;

use tree_sitter::Node;

use crate::parser::ParseContext;

use super::index::JsTypeIndex;
use super::type_helper::JsTypeHelper;

pub(super) struct ExpressionTargetResolver<'a> {
    parser: &'a ParseContext,
    call_node: Node<'a>,
    function_node: Option<Node<'a>>,
    class_name: Option<String>,
    type_index: &'a JsTypeIndex,
    seen: HashSet<String>,
}

impl<'a> ExpressionTargetResolver<'a> {
    fn type_helper(&self) -> JsTypeHelper<'a> {
        JsTypeHelper::new(self.parser)
    }

    pub(super) fn new(
        parser: &'a ParseContext,
        call_node: Node<'a>,
        function_node: Option<Node<'a>>,
        class_name: Option<String>,
        type_index: &'a JsTypeIndex,
    ) -> Self {
        Self {
            parser,
            call_node,
            function_node,
            class_name,
            type_index,
            seen: HashSet::new(),
        }
    }

    fn visit(&mut self, expression_node: Node<'a>) -> Vec<String> {
        let expr_text = self.parser.node_text(expression_node);
        if expr_text.is_empty() || !self.seen.insert(expr_text.clone()) {
            return Vec::new();
        }

        match expression_node.kind() {
            "identifier" | "property_identifier" => self
                .function_node
                .into_iter()
                .flat_map(|function_node| {
                    self.parser
                        .resolve_call_targets(function_node, self.call_node, &expr_text)
                        .into_iter()
                })
                .flat_map(|target| self.resolve_scoped_symbolic_targets(&target))
                .collect(),
            "new_expression" => expression_node
                .child_by_field_name("constructor")
                .and_then(|constructor| self.type_helper().type_name_from_node(constructor))
                .into_iter()
                .collect(),
            "member_expression" => {
                let Some(object_node) = expression_node.child_by_field_name("object") else {
                    return Vec::new();
                };
                let Some(property_node) = expression_node.child_by_field_name("property") else {
                    return Vec::new();
                };
                let property_name = self.parser.node_text(property_node);
                if property_name.is_empty() {
                    return Vec::new();
                }

                if self.parser.node_text_eq(object_node, "this") {
                    return self
                        .class_name
                        .clone()
                        .into_iter()
                        .flat_map(|class_name| {
                            self.class_targets_for_field(&class_name, &property_name)
                        })
                        .collect();
                }

                self.visit(object_node)
                    .into_iter()
                    .flat_map(|class_name| {
                        self.class_targets_for_field(&class_name, &property_name)
                    })
                    .collect()
            }
            "parenthesized_expression" => {
                let mut cursor = expression_node.walk();
                expression_node
                    .named_children(&mut cursor)
                    .flat_map(|child| self.visit(child))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn resolve_expression_targets(mut self, expression_node: Node<'a>) -> Vec<String> {
        let mut targets = self.visit(expression_node);
        targets.sort_unstable();
        targets.dedup();
        targets
    }

    fn resolve_scoped_symbolic_targets(&self, target: &str) -> Vec<String> {
        if target.is_empty() {
            return Vec::new();
        }
        if self.type_index.class_names.contains(target) {
            return vec![target.to_string()];
        }
        if target == "this" {
            return self.class_name.clone().into_iter().collect();
        }
        if let Some(chain) = target.strip_prefix("this.") {
            return self
                .class_name
                .clone()
                .into_iter()
                .flat_map(|class_name| self.resolve_scoped_field_chain(&class_name, chain))
                .collect();
        }
        Vec::new()
    }

    fn resolve_scoped_field_chain(&self, root_class: &str, chain: &str) -> Vec<String> {
        let mut current = vec![root_class.to_string()];
        for segment in chain.split('.') {
            if segment.is_empty() {
                return Vec::new();
            }
            let mut next = Vec::new();
            for class_name in &current {
                next.extend(self.class_targets_for_field(class_name, segment));
            }
            next.sort_unstable();
            next.dedup();
            if next.is_empty() {
                return Vec::new();
            }
            current = next;
        }
        current
    }

    fn class_targets_for_field(&self, class_name: &str, field_name: &str) -> Vec<String> {
        let mut targets: Vec<String> = self
            .type_index
            .field_types_by_class
            .get(class_name)
            .and_then(|fields| fields.get(field_name))
            .cloned()
            .into_iter()
            .flatten()
            .flat_map(|field_type| {
                JsTypeHelper::extract_class_targets(&self.type_index.class_names, &field_type)
            })
            .collect();
        targets.sort_unstable();
        targets.dedup();
        targets
    }
}
