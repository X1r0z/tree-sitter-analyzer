use std::collections::HashMap;
use std::collections::HashSet;

use tree_sitter::Node;

use crate::parser::ParseContext;

use super::resolvers::{JsAliasEvent, JsReceiverEvent};
use super::type_helper::JsTypeHelper;

pub(super) struct JsBindingEventCollector<'a> {
    parser: &'a ParseContext,
    type_helper: JsTypeHelper<'a>,
}

impl<'a> JsBindingEventCollector<'a> {
    pub(super) fn new(parser: &'a ParseContext) -> Self {
        Self {
            parser,
            type_helper: JsTypeHelper::new(parser),
        }
    }

    pub(super) fn collect_alias_events(&self, function_node: Node<'a>) -> Vec<JsAliasEvent> {
        let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
        let mut events = Vec::new();
        Self::walk_function(function_node, false, |node| {
            if node.kind() == "variable_declarator" {
                let Some(name_node) = node.child_by_field_name("name") else {
                    return;
                };
                let Some(value_node) = node.child_by_field_name("value") else {
                    return;
                };
                if name_node.kind() != "identifier" {
                    return;
                }

                let name = self.parser.node_text(name_node);
                let alias = match value_node.kind() {
                    "identifier" | "member_expression" => Self::merge_binding(
                        &mut aliases,
                        Some(name),
                        [self.parser.node_text(value_node)],
                    ),
                    "new_expression" => {
                        let targets = value_node
                            .child_by_field_name("constructor")
                            .and_then(|constructor| {
                                self.type_helper.type_name_from_node(constructor)
                            })
                            .into_iter()
                            .collect::<Vec<_>>();
                        Self::merge_binding(&mut aliases, Some(name), targets)
                    }
                    "array" => {
                        let mut cursor = value_node.walk();
                        let targets = value_node
                            .named_children(&mut cursor)
                            .filter(|child| child.kind() == "identifier")
                            .map(|child| self.parser.node_text(child))
                            .collect::<Vec<_>>();
                        Self::merge_binding(&mut aliases, Some(name), targets)
                    }
                    _ => None,
                };
                if let Some((name, targets)) = alias {
                    events.push(JsAliasEvent {
                        start_byte: node.start_byte(),
                        name,
                        targets,
                    });
                }
                return;
            }

            if node.kind() == "for_in_statement" {
                let Some(left_node) = node.child_by_field_name("left") else {
                    return;
                };
                let Some(right_node) = node.child_by_field_name("right") else {
                    return;
                };
                if right_node.kind() != "identifier" {
                    return;
                }

                let loop_var = self.find_loop_variable_name(left_node);
                let iterable = self.parser.node_text(right_node);
                let alias = if let Some(targets) = aliases.get(&iterable).cloned() {
                    Self::merge_binding(&mut aliases, loop_var, targets)
                } else {
                    Self::merge_binding(&mut aliases, loop_var, [iterable])
                };
                if let Some((name, targets)) = alias {
                    events.push(JsAliasEvent {
                        start_byte: node.start_byte(),
                        name,
                        targets,
                    });
                }
            }
        });
        events
    }

    pub(super) fn collect_receiver_events(
        &self,
        function_node: Node<'a>,
        class_names: &HashSet<String>,
    ) -> Vec<JsReceiverEvent> {
        let mut receivers: HashMap<String, Vec<String>> = HashMap::new();
        let mut events = Vec::new();
        Self::walk_function(function_node, true, |node| {
            if node.kind() != "variable_declarator" {
                return;
            }
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let Some(value_node) = node.child_by_field_name("value") else {
                return;
            };
            if name_node.kind() != "identifier" {
                return;
            }

            let name = self.parser.node_text(name_node);
            let receiver = match value_node.kind() {
                "identifier" => {
                    let value_name = self.parser.node_text(value_node);
                    let targets = if let Some(existing) = receivers.get(&value_name) {
                        existing.clone()
                    } else if class_names.contains(&value_name) {
                        vec![value_name]
                    } else {
                        Vec::new()
                    };
                    Self::overwrite_binding(&mut receivers, Some(name), targets)
                }
                "new_expression" => {
                    let targets = value_node
                        .child_by_field_name("constructor")
                        .and_then(|constructor| self.type_helper.type_name_from_node(constructor))
                        .into_iter()
                        .collect::<Vec<_>>();
                    Self::overwrite_binding(&mut receivers, Some(name), targets)
                }
                "member_expression" => {
                    let value_text = self.parser.node_text(value_node);
                    let targets = value_text
                        .starts_with("this.")
                        .then_some(value_text)
                        .into_iter()
                        .collect::<Vec<_>>();
                    Self::overwrite_binding(&mut receivers, Some(name), targets)
                }
                _ => None,
            };
            if let Some((name, targets)) = receiver {
                events.push(JsReceiverEvent {
                    start_byte: node.start_byte(),
                    name,
                    targets,
                });
            }
        });
        events
    }

    fn walk_function(
        function_node: Node<'a>,
        skip_nested_scopes: bool,
        mut visit: impl FnMut(Node<'a>),
    ) {
        let mut stack = vec![function_node];
        while let Some(node) = stack.pop() {
            visit(node);
            if skip_nested_scopes && Self::is_nested_scope(node, function_node) {
                continue;
            }
            Self::push_children_reversed(node, &mut stack);
        }
    }

    fn is_nested_scope(node: Node<'a>, root: Node<'a>) -> bool {
        matches!(
            node.kind(),
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "class_declaration"
                | "class_expression"
        ) && node.id() != root.id()
    }

    fn push_children_reversed(node: Node<'a>, stack: &mut Vec<Node<'a>>) {
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }

    fn sorted_unique_targets(targets: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut normalized: Vec<String> = targets
            .into_iter()
            .filter(|target| !target.is_empty())
            .collect();
        normalized.sort_unstable();
        normalized.dedup();
        normalized
    }

    fn merge_binding(
        bindings: &mut HashMap<String, Vec<String>>,
        name: Option<String>,
        targets: impl IntoIterator<Item = String>,
    ) -> Option<(String, Vec<String>)> {
        let name = name?;
        if name.is_empty() {
            return None;
        }
        let normalized = Self::sorted_unique_targets(targets);
        if normalized.is_empty() {
            return None;
        }
        let entry = bindings.entry(name.clone()).or_default();
        let mut changed = false;
        for target in normalized {
            if entry.binary_search(&target).is_err() {
                entry.push(target);
                changed = true;
            }
        }
        if !changed {
            return None;
        }
        entry.sort_unstable();
        let targets = if entry.len() == 1 {
            vec![entry[0].clone()]
        } else {
            entry.clone()
        };
        Some((name, targets))
    }

    fn overwrite_binding(
        bindings: &mut HashMap<String, Vec<String>>,
        name: Option<String>,
        targets: impl IntoIterator<Item = String>,
    ) -> Option<(String, Vec<String>)> {
        let name = name?;
        if name.is_empty() {
            return None;
        }
        let normalized = Self::sorted_unique_targets(targets);
        if normalized.is_empty() {
            return None;
        }
        let entry = bindings.entry(name.clone()).or_default();
        if *entry == normalized {
            return None;
        }
        entry.clone_from(&normalized);
        Some((name, normalized))
    }

    fn find_loop_variable_name(&self, node: Node<'a>) -> Option<String> {
        if node.kind() == "identifier" {
            return Some(self.parser.node_text(node));
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "identifier" {
                return Some(self.parser.node_text(child));
            }
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if name_node.kind() == "identifier" {
                        return Some(self.parser.node_text(name_node));
                    }
                }
            }
        }
        None
    }
}
