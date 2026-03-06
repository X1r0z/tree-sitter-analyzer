use std::collections::{HashMap, HashSet, VecDeque};

use tree_sitter::Node;

use super::{BaseParser, JsAliasEvent};
use crate::nodes::FieldInfo;

#[allow(dead_code)]
impl BaseParser {
    pub(crate) fn resolve_js_call_targets_for_identifier(
        &self,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        let Some(func_node) = self.find_enclosing_function_node(call_node) else {
            return Vec::new();
        };

        let events = self.build_js_alias_event_stream(func_node);
        self.resolve_js_call_targets_from_alias_events(
            &events,
            call_node.start_byte(),
            identifier_name,
        )
    }

    pub(crate) fn build_js_alias_event_stream(&self, func_node: Node<'_>) -> Vec<JsAliasEvent> {
        let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
        let mut events = Vec::new();

        fn add_alias(
            aliases: &mut HashMap<String, Vec<String>>,
            name: Option<String>,
            targets: impl IntoIterator<Item = String>,
        ) -> Option<(String, Vec<String>)> {
            let name = name?;
            if name.is_empty() {
                return None;
            }
            let entry = aliases.entry(name.clone()).or_default();
            for target in targets {
                if !target.is_empty() {
                    entry.push(target);
                }
            }
            entry.sort_unstable();
            entry.dedup();
            Some((name, entry.clone()))
        }

        let extract_loop_var = |node: Node<'_>| -> Option<String> {
            if node.kind() == "identifier" {
                return Some(self.node_text(node));
            }
            for i in 0..node.named_child_count() {
                let Some(child) = node.named_child(i as u32) else {
                    continue;
                };
                if child.kind() == "identifier" {
                    return Some(self.node_text(child));
                }
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if name_node.kind() == "identifier" {
                            return Some(self.node_text(name_node));
                        }
                    }
                }
            }
            None
        };

        let mut stack = vec![func_node];
        while let Some(node) = stack.pop() {
            if node.kind() == "variable_declarator" {
                let name_node = node.child_by_field_name("name");
                let value_node = node.child_by_field_name("value");
                if let (Some(name_node), Some(value_node)) = (name_node, value_node) {
                    if name_node.kind() == "identifier" {
                        let name = self.node_text(name_node);
                        if value_node.kind() == "identifier" {
                            if let Some((name, targets)) =
                                add_alias(&mut aliases, Some(name), [self.node_text(value_node)])
                            {
                                events.push(JsAliasEvent {
                                    start_byte: node.start_byte(),
                                    name,
                                    targets,
                                });
                            }
                        } else if value_node.kind() == "array" {
                            let targets = (0..value_node.named_child_count())
                                .filter_map(|i| value_node.named_child(i as u32))
                                .filter(|c| c.kind() == "identifier")
                                .map(|c| self.node_text(c))
                                .collect::<Vec<_>>();
                            if let Some((name, targets)) =
                                add_alias(&mut aliases, Some(name), targets)
                            {
                                events.push(JsAliasEvent {
                                    start_byte: node.start_byte(),
                                    name,
                                    targets,
                                });
                            }
                        }
                    }
                }
            } else if node.kind() == "for_in_statement" {
                let left_node = node.child_by_field_name("left");
                let right_node = node.child_by_field_name("right");
                if let (Some(left_node), Some(right_node)) = (left_node, right_node) {
                    if right_node.kind() == "identifier" {
                        let loop_var = extract_loop_var(left_node);
                        let iterable = self.node_text(right_node);
                        if let Some(targets) = aliases.get(&iterable).cloned() {
                            if let Some((name, targets)) =
                                add_alias(&mut aliases, loop_var, targets)
                            {
                                events.push(JsAliasEvent {
                                    start_byte: node.start_byte(),
                                    name,
                                    targets,
                                });
                            }
                        } else if let Some((name, targets)) =
                            add_alias(&mut aliases, loop_var, [iterable])
                        {
                            events.push(JsAliasEvent {
                                start_byte: node.start_byte(),
                                name,
                                targets,
                            });
                        }
                    }
                }
            }

            for i in (0..node.named_child_count()).rev() {
                if let Some(child) = node.named_child(i as u32) {
                    stack.push(child);
                }
            }
        }

        events
    }

    pub(crate) fn resolve_js_call_targets_from_alias_events(
        &self,
        events: &[JsAliasEvent],
        call_start: usize,
        identifier_name: &str,
    ) -> Vec<String> {
        let mut aliases: HashMap<&str, &[String]> = HashMap::new();
        for event in events {
            if event.start_byte >= call_start {
                break;
            }
            aliases.insert(event.name.as_str(), event.targets.as_slice());
        }

        let mut visited: HashSet<&str> = HashSet::new();
        let mut resolved = Vec::new();
        let mut resolved_seen: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(identifier_name);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(targets) = aliases.get(current) {
                for target in *targets {
                    queue.push_back(target.as_str());
                }
            } else if resolved_seen.insert(current) {
                resolved.push(current);
            }
        }

        resolved.sort_unstable();
        resolved.into_iter().map(str::to_string).collect()
    }

    pub(super) fn extract_js_like_field_infos(
        &self,
        class_node: Node,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        let body = class_node.child_by_field_name("body").or_else(|| {
            for i in 0..class_node.child_count() {
                let child = class_node.child(i as u32).unwrap();
                if child.kind() == "class_body" {
                    return Some(child);
                }
            }
            None
        });
        let Some(body) = body else {
            return Vec::new();
        };

        let mut fields = Vec::new();
        let mut seen = HashSet::new();

        for i in 0..body.child_count() {
            let member = body.child(i as u32).unwrap();
            if !member.is_named() {
                continue;
            }

            if member.kind().ends_with("field_definition")
                || matches!(member.kind(), "property_definition" | "field_definition")
            {
                let name_node = member
                    .child_by_field_name("name")
                    .or_else(|| member.child_by_field_name("property"))
                    .or_else(|| member.child_by_field_name("pattern"));
                let Some(name_node) = name_node else {
                    continue;
                };
                if !matches!(
                    name_node.kind(),
                    "identifier"
                        | "property_identifier"
                        | "private_property_identifier"
                        | "field_identifier"
                ) {
                    continue;
                }
                let name = self.node_text(name_node);
                let field_type = member
                    .child_by_field_name("type")
                    .map(|t| self.normalize_type_text(&self.node_text(t)));
                if !name.is_empty() && seen.insert(name.clone()) {
                    fields.push(FieldInfo {
                        name,
                        location: self.node_location(member),
                        field_type,
                        class_name: Some(class_name.to_string()),
                    });
                }
            }

            if member.kind() != "method_definition" {
                continue;
            }
            let Some(name_node) = member.child_by_field_name("name") else {
                continue;
            };
            if !self.node_eq_str(name_node, "constructor") {
                continue;
            }

            let Some(params) = member.child_by_field_name("parameters") else {
                continue;
            };
            for j in 0..params.child_count() {
                let param = params.child(j as u32).unwrap();
                if !param.is_named() {
                    continue;
                }
                let has_modifier = (0..param.child_count()).any(|k| {
                    let c = param.child(k as u32).unwrap();
                    matches!(c.kind(), "accessibility_modifier" | "readonly")
                });
                if !has_modifier {
                    continue;
                }
                let mut pattern = param
                    .child_by_field_name("pattern")
                    .or_else(|| param.child_by_field_name("name"));
                if let Some(p) = pattern {
                    if p.kind() == "assignment_pattern" {
                        pattern = p.child_by_field_name("left").or(Some(p));
                    }
                }
                let Some(pattern) = pattern else {
                    continue;
                };
                if !matches!(pattern.kind(), "identifier" | "property_identifier") {
                    continue;
                }
                let param_name = self.node_text(pattern);
                let param_type = param
                    .child_by_field_name("type")
                    .map(|t| self.normalize_type_text(&self.node_text(t)));
                if !param_name.is_empty() && seen.insert(param_name.clone()) {
                    fields.push(FieldInfo {
                        name: param_name,
                        location: self.node_location(param),
                        field_type: param_type,
                        class_name: Some(class_name.to_string()),
                    });
                }
            }

            if let Some(body_node) = member.child_by_field_name("body") {
                self.collect_field_infos_from_constructor_body(
                    body_node,
                    class_name,
                    &mut seen,
                    &mut fields,
                );
            }
        }

        fields
    }

    fn collect_field_infos_from_constructor_body(
        &self,
        body_node: Node,
        class_name: &str,
        seen: &mut HashSet<String>,
        fields: &mut Vec<FieldInfo>,
    ) {
        let mut stack = vec![body_node];
        while let Some(node) = stack.pop() {
            if node.kind() == "assignment_expression" {
                if let Some(left) = node.child_by_field_name("left") {
                    if left.kind() == "member_expression" {
                        let obj = left.child_by_field_name("object");
                        let prop = left.child_by_field_name("property");
                        if let (Some(obj), Some(prop)) = (obj, prop) {
                            if self.node_eq_str(obj, "this") {
                                let name = self.node_text(prop);
                                if !name.is_empty() && seen.insert(name.clone()) {
                                    fields.push(FieldInfo {
                                        name,
                                        location: self.node_location(node),
                                        field_type: None,
                                        class_name: Some(class_name.to_string()),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            if matches!(
                node.kind(),
                "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "method_definition"
                    | "class_declaration"
                    | "class_expression"
            ) && node.id() != body_node.id()
            {
                continue;
            }
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
                }
            }
        }
    }

    fn normalize_type_text(&self, text: &str) -> String {
        let stripped = text.trim();
        if let Some(rest) = stripped.strip_prefix(':') {
            rest.trim().to_string()
        } else {
            stripped.to_string()
        }
    }

    pub(super) fn extract_js_like_super_class_names(&self, class_node: Node) -> Vec<String> {
        let mut super_classes = Vec::new();
        let mut seen = HashSet::new();
        self.collect_super_class_names_from_heritage(class_node, &mut super_classes, &mut seen);
        super_classes
    }

    fn collect_super_class_names_from_heritage(
        &self,
        node: Node,
        super_classes: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        for i in 0..node.child_count() {
            let child = node.child(i as u32).unwrap();
            if child.kind() != "class_heritage" {
                continue;
            }
            for j in 0..child.child_count() {
                let sub = child.child(j as u32).unwrap();
                if matches!(sub.kind(), "extends_clause" | "implements_clause") {
                    for k in 0..sub.child_count() {
                        let gc = sub.child(k as u32).unwrap();
                        if gc.is_named() {
                            self.collect_super_class_names_from_expression(gc, super_classes, seen);
                        }
                    }
                } else if sub.is_named() {
                    self.collect_super_class_names_from_expression(sub, super_classes, seen);
                }
            }
        }
    }

    fn collect_super_class_names_from_expression(
        &self,
        node: Node,
        super_classes: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        match node.kind() {
            "identifier" | "type_identifier" | "property_identifier" => {
                let name = self.node_text(node).trim().to_string();
                if !name.is_empty() && seen.insert(name.clone()) {
                    super_classes.push(name);
                }
            }
            "member_expression" => {
                let text = self.node_text(node).trim().to_string();
                if !text.is_empty() && seen.insert(text.clone()) {
                    super_classes.push(text);
                }
                for i in (0..node.child_count()).rev() {
                    let c = node.child(i as u32).unwrap();
                    if matches!(c.kind(), "identifier" | "property_identifier") {
                        let name = self.node_text(c).trim().to_string();
                        if !name.is_empty() && seen.insert(name.clone()) {
                            super_classes.push(name);
                        }
                        break;
                    }
                }
            }
            "expression_with_type_arguments" => {
                if let Some(expr) = node.child_by_field_name("expression") {
                    self.collect_super_class_names_from_expression(expr, super_classes, seen);
                    return;
                }
                for i in 0..node.named_child_count() {
                    if let Some(c) = node.named_child(i as u32) {
                        self.collect_super_class_names_from_expression(c, super_classes, seen);
                        return;
                    }
                }
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    self.collect_super_class_names_from_expression(func, super_classes, seen);
                }
            }
            _ => {
                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child(i as u32) {
                        self.collect_super_class_names_from_expression(child, super_classes, seen);
                    }
                }
            }
        }
    }
}
