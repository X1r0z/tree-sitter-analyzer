use std::collections::HashSet;

use tree_sitter::Node;

use crate::parser::ParseContext;

use super::index::JsTypeIndex;
use super::type_helper::JsTypeHelper;

pub(super) struct ExpressionTargetResolver<'a> {
    ctx: &'a ParseContext,
    call_node: Node<'a>,
    function_node: Option<Node<'a>>,
    class_name: Option<String>,
    type_index: &'a JsTypeIndex,
    seen: HashSet<String>,
}

impl<'a> ExpressionTargetResolver<'a> {
    fn type_helper(&self) -> JsTypeHelper<'a> {
        JsTypeHelper::new(self.ctx)
    }

    pub(super) fn new(
        ctx: &'a ParseContext,
        call_node: Node<'a>,
        function_node: Option<Node<'a>>,
        class_name: Option<String>,
        type_index: &'a JsTypeIndex,
    ) -> Self {
        Self {
            ctx,
            call_node,
            function_node,
            class_name,
            type_index,
            seen: HashSet::new(),
        }
    }

    fn visit_expression(&mut self, expression_node: Node<'a>) -> Vec<String> {
        let expr_text = self.ctx.node_text(expression_node);
        if expr_text.is_empty() || !self.seen.insert(expr_text.clone()) {
            return Vec::new();
        }

        match expression_node.kind() {
            "identifier" | "property_identifier" => self
                .function_node
                .into_iter()
                .flat_map(|function_node| {
                    self.ctx
                        .resolve_call_targets(function_node, self.call_node, &expr_text)
                        .into_iter()
                })
                .flat_map(|target| self.symbolic_targets_via_type_index(&target))
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
                let property_name = self.ctx.node_text(property_node);
                if property_name.is_empty() {
                    return Vec::new();
                }

                if self.ctx.node_text_eq(object_node, "this") {
                    return self
                        .class_name
                        .clone()
                        .into_iter()
                        .flat_map(|class_name| {
                            self.field_class_targets(&class_name, &property_name)
                        })
                        .collect();
                }

                self.visit_expression(object_node)
                    .into_iter()
                    .flat_map(|class_name| {
                        self.field_class_targets(&class_name, &property_name)
                    })
                    .collect()
            }
            "parenthesized_expression" => {
                let mut cursor = expression_node.walk();
                expression_node
                    .named_children(&mut cursor)
                    .flat_map(|child| self.visit_expression(child))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn resolve_expression_targets(mut self, expression_node: Node<'a>) -> Vec<String> {
        let mut targets = self.visit_expression(expression_node);
        targets.sort_unstable();
        targets.dedup();
        targets
    }

    fn symbolic_targets_via_type_index(&self, target: &str) -> Vec<String> {
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
                .flat_map(|class_name| self.field_chain_via_type_index(&class_name, chain))
                .collect();
        }
        Vec::new()
    }

    fn field_chain_via_type_index(&self, root_class: &str, chain: &str) -> Vec<String> {
        let mut current = vec![root_class.to_string()];
        for segment in chain.split('.') {
            if segment.is_empty() {
                return Vec::new();
            }
            let mut next = Vec::new();
            for class_name in &current {
                next.extend(self.field_class_targets(class_name, segment));
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

    fn field_class_targets(&self, class_name: &str, field_name: &str) -> Vec<String> {
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
