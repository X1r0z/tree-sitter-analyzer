use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use tree_sitter::Node;

use crate::languages::QueryKind;
use crate::models::{FieldInfo, FunctionParamInfo};
use crate::parser::capture;
use crate::parser::ParseContext;

use super::expression::ExpressionTargetResolver;
use super::index::JsTypeIndex;
use super::type_helper::JsTypeHelper;

#[derive(Debug, Clone, Default)]
pub(crate) struct JsSemanticFacts {
    pub(crate) class_names: HashSet<String>,
    pub(crate) constructor_arg_types: HashMap<(String, usize), Option<String>>,
    pub(crate) field_types_by_class: HashMap<String, HashMap<String, Option<String>>>,
    pub(crate) field_infos_by_class: HashMap<String, Vec<FieldInfo>>,
}

#[derive(Clone)]
struct PendingFieldInference {
    class_name: String,
    field_name: String,
    location: crate::models::Location,
    param_index: usize,
}

struct JsSemanticFactsBuilder<'a> {
    ctx: &'a ParseContext,
    facts: JsSemanticFacts,
    pending_fields: Vec<PendingFieldInference>,
}

#[derive(Clone)]
struct TraversalContext<'a> {
    function_node: Option<Node<'a>>,
    class_name: Option<String>,
}

impl<'a> JsSemanticFactsBuilder<'a> {
    fn type_helper(&self) -> JsTypeHelper<'a> {
        JsTypeHelper::new(self.ctx)
    }

    fn new(ctx: &'a ParseContext) -> Self {
        let class_names = capture::collect_capture_pairs(ctx, QueryKind::Class, "class", "name")
            .into_iter()
            .map(|(_, name_node)| ctx.node_text(name_node))
            .filter(|name| !name.is_empty())
            .collect::<HashSet<_>>();
        Self {
            ctx,
            facts: JsSemanticFacts {
                class_names,
                ..JsSemanticFacts::default()
            },
            pending_fields: Vec::new(),
        }
    }

    fn build(mut self) -> JsSemanticFacts {
        for (class_node, name_node) in
            capture::collect_capture_pairs(self.ctx, QueryKind::Class, "class", "name")
        {
            let class_name = self.ctx.node_text(name_node);
            if class_name.is_empty() {
                continue;
            }
            let class_node = self
                .ctx
                .engine()
                .normalize_class_node(self.ctx, class_node);
            self.collect_class_facts(class_node, &class_name);
        }

        if !self.pending_fields.is_empty() {
            self.infer_constructor_arg_types();

            for pending in std::mem::take(&mut self.pending_fields) {
                let inferred_type = self
                    .facts
                    .constructor_arg_types
                    .get(&(pending.class_name.clone(), pending.param_index))
                    .cloned()
                    .unwrap_or(None);
                self.upsert_field_fact(
                    &pending.class_name,
                    &pending.field_name,
                    pending.location,
                    inferred_type,
                );
            }
        }

        self.facts
    }

    fn collect_class_facts(&mut self, class_node: Node<'a>, class_name: &str) {
        let body = class_node.child_by_field_name("body").or_else(|| {
            let mut cursor = class_node.walk();
            let body = class_node
                .children(&mut cursor)
                .find(|child| child.kind() == "class_body");
            body
        });
        let Some(body) = body else {
            return;
        };

        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            if !member.is_named() {
                continue;
            }

            if member.kind().ends_with("field_definition")
                || matches!(member.kind(), "property_definition" | "field_definition")
            {
                self.collect_declared_field_facts(member, class_name);
                continue;
            }

            if member.kind() != "method_definition" {
                continue;
            }
            let Some(name_node) = member.child_by_field_name("name") else {
                continue;
            };
            if !self.ctx.node_text_eq(name_node, "constructor") {
                continue;
            }
            let Some(params) = member.child_by_field_name("parameters") else {
                continue;
            };
            let constructor_params = self.type_helper().collect_constructor_params(params);
            self.collect_param_property_facts(params, class_name);
            if let Some(body_node) = member.child_by_field_name("body") {
                self.collect_constructor_assignment_facts(
                    body_node,
                    class_name,
                    &constructor_params,
                );
            }
        }
    }

    fn collect_declared_field_facts(&mut self, member: Node<'a>, class_name: &str) {
        let name_node = member
            .child_by_field_name("name")
            .or_else(|| member.child_by_field_name("property"))
            .or_else(|| member.child_by_field_name("pattern"));
        let Some(name_node) = name_node else {
            return;
        };
        if !matches!(
            name_node.kind(),
            "identifier"
                | "property_identifier"
                | "private_property_identifier"
                | "field_identifier"
        ) {
            return;
        }
        let name = self.ctx.node_text(name_node);
        if name.is_empty() {
            return;
        }
        let type_helper = self.type_helper();
        let field_type = member
            .child_by_field_name("type")
            .map(|type_node| JsTypeHelper::normalize_type_text(&self.ctx.node_text(type_node)))
            .or_else(|| {
                member
                    .child_by_field_name("value")
                    .and_then(|value| type_helper.field_type_from_value(value))
            })
            .or_else(|| type_helper.field_type_from_member(member, name_node.id()));
        self.upsert_field_fact(
            class_name,
            &name,
            self.ctx.node_location(member),
            field_type,
        );
    }

    fn collect_param_property_facts(&mut self, params: Node<'a>, class_name: &str) {
        let mut cursor = params.walk();
        for param in params.children(&mut cursor) {
            if !param.is_named() {
                continue;
            }
            let mut param_cursor = param.walk();
            let has_modifier = param
                .children(&mut param_cursor)
                .any(|child| matches!(child.kind(), "accessibility_modifier" | "readonly"));
            if !has_modifier {
                continue;
            }
            let mut pattern = param
                .child_by_field_name("pattern")
                .or_else(|| param.child_by_field_name("name"));
            if let Some(pattern_node) = pattern {
                if pattern_node.kind() == "assignment_pattern" {
                    pattern = pattern_node
                        .child_by_field_name("left")
                        .or(Some(pattern_node));
                }
            }
            let Some(pattern) = pattern else {
                continue;
            };
            if !matches!(pattern.kind(), "identifier" | "property_identifier") {
                continue;
            }
            let param_name = self.ctx.node_text(pattern);
            if param_name.is_empty() {
                continue;
            }
            let param_type = param.child_by_field_name("type").map(|type_node| {
                JsTypeHelper::normalize_type_text(&self.ctx.node_text(type_node))
            });
            self.upsert_field_fact(
                class_name,
                &param_name,
                self.ctx.node_location(param),
                param_type,
            );
        }
    }

    fn collect_constructor_assignment_facts(
        &mut self,
        body_node: Node<'a>,
        class_name: &str,
        constructor_params: &[FunctionParamInfo],
    ) {
        let mut stack = vec![body_node];
        while let Some(node) = stack.pop() {
            if node.kind() == "assignment_expression" {
                let Some(left) = node.child_by_field_name("left") else {
                    continue;
                };
                if left.kind() != "member_expression" {
                    continue;
                }
                let Some(obj) = left.child_by_field_name("object") else {
                    continue;
                };
                let Some(prop) = left.child_by_field_name("property") else {
                    continue;
                };
                if !self.ctx.node_text_eq(obj, "this") {
                    continue;
                }

                let name = self.ctx.node_text(prop);
                if name.is_empty() {
                    continue;
                }

                let location = self.ctx.node_location(node);
                let right = node.child_by_field_name("right");
                let type_helper = self.type_helper();
                let field_type = right.and_then(|value| {
                    type_helper.field_type_from_value(value).or_else(|| {
                        type_helper.field_type_from_constructor_param(
                            value,
                            class_name,
                            constructor_params,
                            &self.facts.constructor_arg_types,
                        )
                    })
                });
                if field_type.is_some() {
                    self.upsert_field_fact(class_name, &name, location, field_type);
                    continue;
                }

                let param_index = right
                    .filter(|value_node| value_node.kind() == "identifier")
                    .and_then(|value_node| {
                        let param_name = self.ctx.node_text(value_node);
                        constructor_params
                            .iter()
                            .position(|param| param.name == param_name)
                    });
                if let Some(param_index) = param_index {
                    self.pending_fields.push(PendingFieldInference {
                        class_name: class_name.to_string(),
                        field_name: name,
                        location,
                        param_index,
                    });
                } else {
                    self.upsert_field_fact(class_name, &name, location, None);
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
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
    }

    fn infer_constructor_arg_types(&mut self) {
        let mut required_params: HashMap<String, HashSet<usize>> = HashMap::new();
        for pending in &self.pending_fields {
            required_params
                .entry(pending.class_name.clone())
                .or_default()
                .insert(pending.param_index);
        }
        if required_params.is_empty() {
            return;
        }

        let type_index = JsTypeIndex {
            class_names: self.facts.class_names.clone(),
            field_types_by_class: self.facts.field_types_by_class.clone(),
            field_infos_by_class: self.facts.field_infos_by_class.clone(),
        };
        let mut candidates_by_param: HashMap<(String, usize), Vec<String>> = HashMap::new();
        let mut stack = vec![(
            self.ctx.tree().root_node(),
            TraversalContext {
                function_node: None,
                class_name: None,
            },
        )];

        while let Some((node, context)) = stack.pop() {
            if node.kind() == "new_expression" {
                let type_helper = self.type_helper();
                let constructor_name = node
                    .child_by_field_name("constructor")
                    .and_then(|constructor| type_helper.type_name_from_node(constructor));
                if let Some(class_name) = constructor_name {
                    if let Some(required_indexes) = required_params.get(&class_name) {
                        if let Some(arguments) = node.child_by_field_name("arguments") {
                            for &param_index in required_indexes {
                                let mut cursor = arguments.walk();
                                let Some(argument) =
                                    arguments.named_children(&mut cursor).nth(param_index)
                                else {
                                    continue;
                                };
                                let mut targets = ExpressionTargetResolver::new(
                                    self.ctx,
                                    node,
                                    context.function_node,
                                    context.class_name.clone(),
                                    &type_index,
                                )
                                .resolve(argument);
                                targets.sort_unstable();
                                targets.dedup();
                                if targets.len() == 1 {
                                    let target = targets.pop().expect("single target");
                                    candidates_by_param
                                        .entry((class_name.clone(), param_index))
                                        .or_default()
                                        .push(target);
                                }
                            }
                        }
                    }
                }
            }

            let mut child_context = context;
            if matches!(node.kind(), "class_declaration" | "class_expression") {
                child_context.class_name = self.ctx.cached_class_name(node);
            }
            if matches!(
                node.kind(),
                "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "method_definition"
            ) {
                child_context.function_node = Some(node);
            }
            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push((child, child_context.clone()));
            }
        }

        for (key, mut candidates) in candidates_by_param {
            candidates.sort_unstable();
            candidates.dedup();
            let resolved = (candidates.len() == 1).then(|| candidates[0].clone());
            self.facts.constructor_arg_types.insert(key, resolved);
        }
    }

    fn upsert_field_fact(
        &mut self,
        class_name: &str,
        field_name: &str,
        location: crate::models::Location,
        field_type: Option<String>,
    ) {
        let class_entry = self
            .facts
            .field_types_by_class
            .entry(class_name.to_string())
            .or_default();
        class_entry
            .entry(field_name.to_string())
            .or_insert_with(|| field_type.clone());

        let fields = self
            .facts
            .field_infos_by_class
            .entry(class_name.to_string())
            .or_default();
        if let Some(existing) = fields.iter_mut().find(|field| field.name == field_name) {
            if existing.field_type.is_none() && field_type.is_some() {
                existing.field_type = field_type;
            }
            return;
        }
        fields.push(FieldInfo {
            name: field_name.to_string(),
            location,
            field_type,
            class_name: Some(class_name.to_string()),
        });
    }
}

pub(crate) fn semantic_facts(ctx: &ParseContext) -> Rc<JsSemanticFacts> {
    if let Some(cached) = ctx.caches.borrow().language.js.semantic_facts.as_ref() {
        return Rc::clone(cached);
    }

    let facts = Rc::new(JsSemanticFactsBuilder::new(ctx).build());
    ctx.caches.borrow_mut().language.js.semantic_facts = Some(Rc::clone(&facts));
    facts
}

pub(super) fn class_names(ctx: &ParseContext) -> Rc<HashSet<String>> {
    if let Some(cached) = ctx.caches.borrow().language.js.class_names.as_ref() {
        return Rc::clone(cached);
    }

    let class_names = Rc::new(semantic_facts(ctx).class_names.clone());
    ctx.caches.borrow_mut().language.js.class_names = Some(Rc::clone(&class_names));
    class_names
}
