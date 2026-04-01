use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use tree_sitter::Node;

use super::super::capture::{self, CallCaptureMatch};
use super::super::ParseContext;
use crate::languages::{find_language_info, LanguageEngine, LanguageInfo, QueryKind, ResolvedCall};
use crate::models::{FieldInfo, FunctionParamInfo};
use crate::traversal::collect_reachable_bfs;

pub(crate) struct JavaScriptFamilyEngine {
    language: &'static str,
}

#[derive(Clone)]
pub(crate) struct JsAliasEvent {
    pub(crate) start_byte: usize,
    pub(crate) name: String,
    pub(crate) targets: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct JsTypeFacts {
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

struct JsTypeFactsBuilder<'a> {
    parser: &'a ParseContext,
    facts: JsTypeFacts,
    pending_fields: Vec<PendingFieldInference>,
}

struct JsAliasResolver<'a> {
    parser: &'a ParseContext,
    function_node: Node<'a>,
    call_node: Node<'a>,
}

struct JsTypeHelper<'a> {
    parser: &'a ParseContext,
}

struct ExpressionTargetResolver<'a> {
    parser: &'a ParseContext,
    call_node: Node<'a>,
    facts: &'a JsTypeFacts,
    seen: HashSet<String>,
}

pub(crate) static JAVASCRIPT_ENGINE: JavaScriptFamilyEngine = JavaScriptFamilyEngine {
    language: "javascript",
};
pub(crate) static TYPESCRIPT_ENGINE: JavaScriptFamilyEngine = JavaScriptFamilyEngine {
    language: "typescript",
};
pub(crate) static TSX_ENGINE: JavaScriptFamilyEngine = JavaScriptFamilyEngine { language: "tsx" };

impl LanguageEngine for JavaScriptFamilyEngine {
    fn language_info(&self) -> &'static LanguageInfo {
        find_language_info(self.language).expect("javascript family language info")
    }

    fn function_name(&self, ctx: &ParseContext, node: Node<'_>) -> Option<String> {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = ctx.node_text(name_node);
            if !name.is_empty() {
                return Some(name);
            }
        }
        if matches!(node.kind(), "arrow_function" | "function_expression") {
            return anonymous_function_name(ctx, node);
        }
        for i in 0..node.child_count() {
            let child = node.child(i as u32).unwrap();
            if matches!(
                child.kind(),
                "identifier" | "property_identifier" | "field_identifier"
            ) {
                let name = ctx.node_text(child);
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
        None
    }

    fn function_params(
        &self,
        ctx: &ParseContext,
        function_node: Node<'_>,
    ) -> Vec<FunctionParamInfo> {
        let type_helper = JsTypeHelper::new(ctx);
        let Some(parameters) = function_node.child_by_field_name("parameters") else {
            return Vec::new();
        };

        let mut params = Vec::new();
        for i in 0..parameters.named_child_count() {
            let Some(param) = parameters.named_child(i as u32) else {
                continue;
            };
            if let Some(info) = type_helper.build_parameter_info(param) {
                params.push(info);
            }
        }
        params
    }

    fn resolve_call<'a>(
        &self,
        ctx: &ParseContext,
        matched: &CallCaptureMatch<'a>,
    ) -> ResolvedCall<'a> {
        let call_node = matched.call;
        let mut callee = String::new();
        let mut is_method = false;
        let mut object_name: Option<String> = None;
        let mut callee_function_node: Option<Node<'_>> = None;

        match call_node.kind() {
            "call_expression" => {
                if let Some(func_node) = call_node.child_by_field_name("function") {
                    if func_node.kind() == "super" {
                        callee = "constructor".to_string();
                        object_name = Some("super()".to_string());
                    } else {
                        callee_function_node = Some(func_node);
                        if func_node.kind() == "identifier" {
                            callee = ctx.node_text(func_node);
                        } else if func_node.kind() == "member_expression" {
                            is_method = true;
                            let (callee_name, resolved_object_name) =
                                split_attribute_parts(ctx, func_node);
                            callee = callee_name;
                            object_name = func_node
                                .child_by_field_name("object")
                                .and_then(|object_node| {
                                    resolve_call_object_name(ctx, call_node, object_node)
                                })
                                .or(resolved_object_name);
                        }
                    }
                }
            }
            "new_expression" => {
                if let Some(constructor_node) = call_node.child_by_field_name("constructor") {
                    callee = "constructor".to_string();
                    object_name = Some(ctx.node_text(constructor_node));
                }
            }
            _ => {}
        }
        if callee.is_empty() {
            if let Some(callee_cap) = matched.callee {
                callee = ctx.node_text(callee_cap);
            }
            if let Some(method_cap) = matched.method {
                callee = ctx.node_text(method_cap);
                is_method = true;
                if let Some(obj_cap) = matched.object {
                    object_name = Some(ctx.node_text(obj_cap));
                }
            }
        }

        ResolvedCall {
            callee,
            is_method,
            object_name,
            callee_function_node,
        }
    }

    fn is_ref_node(&self, node: Node<'_>) -> bool {
        matches!(
            node.kind(),
            "identifier"
                | "property_identifier"
                | "private_property_identifier"
                | "field_identifier"
                | "type_identifier"
        )
    }

    fn class_fields(
        &self,
        ctx: &ParseContext,
        _class_node: Node<'_>,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        type_facts(ctx)
            .field_infos_by_class
            .get(class_name)
            .cloned()
            .unwrap_or_default()
    }

    fn super_types(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
        let mut super_classes = Vec::new();
        let mut seen = HashSet::new();
        for i in 0..class_node.child_count() {
            let child = class_node.child(i as u32).unwrap();
            if child.kind() != "class_heritage" {
                continue;
            }
            for j in 0..child.child_count() {
                let sub = child.child(j as u32).unwrap();
                if matches!(sub.kind(), "extends_clause" | "implements_clause") {
                    for k in 0..sub.child_count() {
                        let grandchild = sub.child(k as u32).unwrap();
                        if grandchild.is_named() {
                            collect_super_types_from_expr(
                                ctx,
                                grandchild,
                                &mut super_classes,
                                &mut seen,
                            );
                        }
                    }
                } else if sub.is_named() {
                    collect_super_types_from_expr(ctx, sub, &mut super_classes, &mut seen);
                }
            }
        }
        super_classes
    }
}

impl<'a> JsTypeFactsBuilder<'a> {
    fn type_helper(&self) -> JsTypeHelper<'a> {
        JsTypeHelper::new(self.parser)
    }

    fn new(parser: &'a ParseContext) -> Self {
        let class_names = capture::collect_capture_pairs(parser, QueryKind::Class, "class", "name")
            .into_iter()
            .map(|(_, name_node)| parser.node_text(name_node))
            .filter(|name| !name.is_empty())
            .collect::<HashSet<_>>();
        Self {
            parser,
            facts: JsTypeFacts {
                class_names,
                ..JsTypeFacts::default()
            },
            pending_fields: Vec::new(),
        }
    }

    fn build(mut self) -> JsTypeFacts {
        for (class_node, name_node) in
            capture::collect_capture_pairs(self.parser, QueryKind::Class, "class", "name")
        {
            let class_name = self.parser.node_text(name_node);
            if class_name.is_empty() {
                continue;
            }
            let class_node = self
                .parser
                .engine()
                .normalize_class_node(self.parser, class_node);
            self.collect_class_facts(class_node, &class_name);
        }

        self.collect_constructor_arg_types();

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

        self.facts
    }

    fn collect_class_facts(&mut self, class_node: Node<'a>, class_name: &str) {
        let body = class_node.child_by_field_name("body").or_else(|| {
            (0..class_node.child_count())
                .filter_map(|i| class_node.child(i as u32))
                .find(|child| child.kind() == "class_body")
        });
        let Some(body) = body else {
            return;
        };

        for i in 0..body.child_count() {
            let Some(member) = body.child(i as u32) else {
                continue;
            };
            if !member.is_named() {
                continue;
            }

            if member.kind().ends_with("field_definition")
                || matches!(member.kind(), "property_definition" | "field_definition")
            {
                self.collect_declared_field_fact(member, class_name);
                continue;
            }

            if member.kind() != "method_definition" {
                continue;
            }
            let Some(name_node) = member.child_by_field_name("name") else {
                continue;
            };
            if !self.parser.node_text_eq(name_node, "constructor") {
                continue;
            }
            let Some(params) = member.child_by_field_name("parameters") else {
                continue;
            };
            let constructor_params = self.type_helper().collect_constructor_params(params);
            self.collect_constructor_param_facts(params, class_name);
            if let Some(body_node) = member.child_by_field_name("body") {
                self.collect_constructor_field_facts(body_node, class_name, &constructor_params);
            }
        }
    }

    fn collect_declared_field_fact(&mut self, member: Node<'a>, class_name: &str) {
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
        let name = self.parser.node_text(name_node);
        if name.is_empty() {
            return;
        }
        let type_helper = self.type_helper();
        let field_type = member
            .child_by_field_name("type")
            .map(|type_node| type_helper.normalize_type_text(&self.parser.node_text(type_node)))
            .or_else(|| {
                member
                    .child_by_field_name("value")
                    .and_then(|value| type_helper.field_type_from_value(value))
            })
            .or_else(|| type_helper.field_type_from_member(member, name_node.id()));
        self.upsert_field_fact(
            class_name,
            &name,
            self.parser.node_location(member),
            field_type,
        );
    }

    fn collect_constructor_param_facts(&mut self, params: Node<'a>, class_name: &str) {
        for j in 0..params.child_count() {
            let Some(param) = params.child(j as u32) else {
                continue;
            };
            if !param.is_named() {
                continue;
            }
            let has_modifier = (0..param.child_count()).any(|k| {
                let child = param.child(k as u32).unwrap();
                matches!(child.kind(), "accessibility_modifier" | "readonly")
            });
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
            let param_name = self.parser.node_text(pattern);
            if param_name.is_empty() {
                continue;
            }
            let param_type = param.child_by_field_name("type").map(|type_node| {
                self.type_helper()
                    .normalize_type_text(&self.parser.node_text(type_node))
            });
            self.upsert_field_fact(
                class_name,
                &param_name,
                self.parser.node_location(param),
                param_type,
            );
        }
    }

    fn collect_constructor_field_facts(
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
                if !self.parser.node_text_eq(obj, "this") {
                    continue;
                }

                let name = self.parser.node_text(prop);
                if name.is_empty() {
                    continue;
                }

                let location = self.parser.node_location(node);
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
                        let param_name = self.parser.node_text(value_node);
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
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
                }
            }
        }
    }

    fn collect_constructor_arg_types(&mut self) {
        let mut candidates_by_param: HashMap<(String, usize), Vec<String>> = HashMap::new();
        let mut stack = vec![self.parser.tree().root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "new_expression" {
                let type_helper = self.type_helper();
                let constructor_name = node
                    .child_by_field_name("constructor")
                    .and_then(|constructor| type_helper.type_name_from_node(constructor));
                if let Some(class_name) = constructor_name {
                    if self.facts.class_names.contains(&class_name) {
                        if let Some(arguments) = node.child_by_field_name("arguments") {
                            for i in 0..arguments.named_child_count() {
                                let Some(argument) = arguments.named_child(i as u32) else {
                                    continue;
                                };
                                let mut targets =
                                    ExpressionTargetResolver::new(self.parser, node, &self.facts)
                                        .resolve_expression_targets(argument);
                                targets.sort_unstable();
                                targets.dedup();
                                if targets.len() == 1 {
                                    let target = targets.pop().expect("single target");
                                    candidates_by_param
                                        .entry((class_name.clone(), i))
                                        .or_default()
                                        .push(target);
                                }
                            }
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

impl<'a> JsAliasResolver<'a> {
    fn new(parser: &'a ParseContext, function_node: Node<'a>, call_node: Node<'a>) -> Self {
        Self {
            parser,
            function_node,
            call_node,
        }
    }

    fn resolve_identifier_targets(&self, identifier_name: &str) -> Vec<String> {
        let mut caches = self.parser.caches.borrow_mut();
        let function_id = self.parser.node_id(self.function_node);
        let cache_key = (
            function_id,
            self.call_node.start_byte(),
            identifier_name.to_string(),
        );
        if let Some(cached) = caches.language.js.identifier_targets.get(&cache_key) {
            return cached.clone();
        }

        let events = caches
            .language
            .js
            .alias_events_by_function
            .entry(function_id)
            .or_insert_with(|| self.collect_alias_events())
            .clone();
        drop(caches);

        let resolved = self.resolve_from_events(&events, identifier_name);
        self.parser
            .caches
            .borrow_mut()
            .language
            .js
            .identifier_targets
            .insert(cache_key, resolved.clone());
        resolved
    }

    fn collect_alias_events(&self) -> Vec<JsAliasEvent> {
        let type_helper = JsTypeHelper::new(self.parser);
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

        let find_loop_var_name = |node: Node<'_>| -> Option<String> {
            if node.kind() == "identifier" {
                return Some(self.parser.node_text(node));
            }
            for i in 0..node.named_child_count() {
                let Some(child) = node.named_child(i as u32) else {
                    continue;
                };
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
        };

        let mut stack = vec![self.function_node];
        while let Some(node) = stack.pop() {
            if node.kind() == "variable_declarator" {
                let Some(name_node) = node.child_by_field_name("name") else {
                    continue;
                };
                let Some(value_node) = node.child_by_field_name("value") else {
                    continue;
                };
                if name_node.kind() != "identifier" {
                    continue;
                }

                let name = self.parser.node_text(name_node);
                let alias = match value_node.kind() {
                    "identifier" => add_alias(
                        &mut aliases,
                        Some(name),
                        [self.parser.node_text(value_node)],
                    ),
                    "new_expression" => {
                        let targets = value_node
                            .child_by_field_name("constructor")
                            .and_then(|constructor| type_helper.type_name_from_node(constructor))
                            .into_iter()
                            .collect::<Vec<_>>();
                        add_alias(&mut aliases, Some(name), targets)
                    }
                    "member_expression" => add_alias(
                        &mut aliases,
                        Some(name),
                        [self.parser.node_text(value_node)],
                    ),
                    "array" => {
                        let targets = (0..value_node.named_child_count())
                            .filter_map(|i| value_node.named_child(i as u32))
                            .filter(|child| child.kind() == "identifier")
                            .map(|child| self.parser.node_text(child))
                            .collect::<Vec<_>>();
                        add_alias(&mut aliases, Some(name), targets)
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
            } else if node.kind() == "for_in_statement" {
                let Some(left_node) = node.child_by_field_name("left") else {
                    continue;
                };
                let Some(right_node) = node.child_by_field_name("right") else {
                    continue;
                };
                if right_node.kind() != "identifier" {
                    continue;
                }

                let loop_var = find_loop_var_name(left_node);
                let iterable = self.parser.node_text(right_node);
                let alias = if let Some(targets) = aliases.get(&iterable).cloned() {
                    add_alias(&mut aliases, loop_var, targets)
                } else {
                    add_alias(&mut aliases, loop_var, [iterable])
                };
                if let Some((name, targets)) = alias {
                    events.push(JsAliasEvent {
                        start_byte: node.start_byte(),
                        name,
                        targets,
                    });
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

    fn resolve_from_events(&self, events: &[JsAliasEvent], identifier_name: &str) -> Vec<String> {
        let mut active_aliases: HashMap<&str, &[String]> = HashMap::new();
        for event in events {
            if event.start_byte >= self.call_node.start_byte() {
                break;
            }
            active_aliases.insert(event.name.as_str(), event.targets.as_slice());
        }

        let mut resolved: Vec<String> = collect_reachable_bfs(
            [identifier_name],
            [identifier_name],
            |current| {
                active_aliases
                    .get(current)
                    .map(|targets| targets.iter().map(|target| target.as_str()).collect())
                    .unwrap_or_default()
            },
            |next| *next,
            |next| *next,
        )
        .into_iter()
        .map(str::to_string)
        .collect();
        if !active_aliases.contains_key(identifier_name) {
            resolved.push(identifier_name.to_string());
        }
        resolved.sort_unstable();
        resolved.dedup();
        resolved
    }
}

impl<'a> JsTypeHelper<'a> {
    fn new(parser: &'a ParseContext) -> Self {
        Self { parser }
    }

    fn field_type_from_value(&self, value_node: Node<'_>) -> Option<String> {
        match value_node.kind() {
            "call_expression" => {
                let function = value_node.child_by_field_name("function")?;
                let arguments = value_node.child_by_field_name("arguments")?;
                let is_supported_factory = matches!(
                    function.kind(),
                    "identifier" | "property_identifier" | "member_expression"
                ) && matches!(
                    self.parser.node_text(function).as_str(),
                    "inject" | "forwardRef" | "signal" | "computed"
                );
                if !is_supported_factory {
                    return None;
                }
                self.find_first_type_name(arguments)
            }
            "new_expression" => value_node
                .child_by_field_name("constructor")
                .and_then(|constructor| self.type_name_from_node(constructor)),
            _ => None,
        }
    }

    fn field_type_from_member(&self, member: Node<'_>, name_node_id: usize) -> Option<String> {
        for i in 0..member.named_child_count() {
            let child = member.named_child(i as u32)?;
            if child.id() == name_node_id || child.kind().ends_with("modifier") {
                continue;
            }
            if let Some(field_type) = self.field_type_from_value(child) {
                return Some(field_type);
            }
        }
        None
    }

    fn field_type_from_constructor_param(
        &self,
        value_node: Node<'_>,
        class_name: &str,
        constructor_params: &[FunctionParamInfo],
        constructor_arg_types: &HashMap<(String, usize), Option<String>>,
    ) -> Option<String> {
        if value_node.kind() != "identifier" {
            return None;
        }
        let param_name = self.parser.node_text(value_node);
        if param_name.is_empty() {
            return None;
        }

        if let Some(param_type) = constructor_params
            .iter()
            .find(|param| param.name == param_name)
            .and_then(|param| param.param_type.clone())
        {
            return Some(param_type);
        }

        let param_index = constructor_params
            .iter()
            .position(|param| param.name == param_name)?;
        constructor_arg_types
            .get(&(class_name.to_string(), param_index))
            .cloned()
            .unwrap_or(None)
    }

    fn find_first_type_name(&self, node: Node<'_>) -> Option<String> {
        for i in 0..node.named_child_count() {
            let child = node.named_child(i as u32)?;
            if let Some(name) = self.type_name_from_node(child) {
                return Some(name);
            }
        }
        None
    }

    fn type_name_from_node(&self, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "identifier" | "type_identifier" => {
                let name = self.parser.node_text(node);
                (!name.is_empty()).then_some(name)
            }
            "member_expression" => node
                .child_by_field_name("property")
                .and_then(|property| self.type_name_from_node(property)),
            "type_arguments" | "arguments" | "parenthesized_expression" => {
                self.find_first_type_name(node)
            }
            _ => None,
        }
    }

    fn normalize_type_text(&self, text: &str) -> String {
        Self::normalize_type_text_value(text)
    }

    fn normalize_type_text_value(text: &str) -> String {
        let stripped = text.trim();
        if let Some(rest) = stripped.strip_prefix(':') {
            rest.trim().to_string()
        } else {
            stripped.to_string()
        }
    }

    fn build_parameter_info(&self, param: Node<'_>) -> Option<FunctionParamInfo> {
        let name_node = Self::find_param_name_node(param)?;
        let name = self.parser.node_text(name_node).trim().to_string();
        if name.is_empty() {
            return None;
        }

        Some(FunctionParamInfo {
            name,
            param_type: self.find_param_type(param),
        })
    }

    fn find_param_name_node<'b>(node: Node<'b>) -> Option<Node<'b>> {
        match node.kind() {
            "identifier"
            | "property_identifier"
            | "private_property_identifier"
            | "object_pattern"
            | "array_pattern" => Some(node),
            "assignment_pattern" => node
                .child_by_field_name("left")
                .and_then(Self::find_param_name_node),
            "rest_pattern" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    return Self::find_param_name_node(pattern);
                }
                node.named_child(0).and_then(Self::find_param_name_node)
            }
            _ => node
                .child_by_field_name("pattern")
                .or_else(|| node.child_by_field_name("name"))
                .and_then(Self::find_param_name_node)
                .or_else(|| node.named_child(0).and_then(Self::find_param_name_node)),
        }
    }

    fn find_param_type(&self, node: Node<'_>) -> Option<String> {
        if let Some(type_node) = node.child_by_field_name("type") {
            return Some(self.normalize_type_text(&self.parser.node_text(type_node)));
        }

        node.child_by_field_name("pattern")
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| node.child_by_field_name("left"))
            .and_then(|child| self.find_param_type(child))
    }

    fn collect_constructor_params(&self, params_node: Node<'_>) -> Vec<FunctionParamInfo> {
        let mut params = Vec::new();
        for i in 0..params_node.named_child_count() {
            let Some(param) = params_node.named_child(i as u32) else {
                continue;
            };
            if let Some(info) = self.build_parameter_info(param) {
                params.push(info);
            }
        }
        params
    }
}

impl<'a> ExpressionTargetResolver<'a> {
    fn type_helper(&self) -> JsTypeHelper<'a> {
        JsTypeHelper::new(self.parser)
    }

    fn new(parser: &'a ParseContext, call_node: Node<'a>, facts: &'a JsTypeFacts) -> Self {
        Self {
            parser,
            call_node,
            facts,
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
                .parser
                .resolve_call_targets_for_identifier(self.call_node, &expr_text)
                .into_iter()
                .flat_map(|target| self.resolve_symbolic_class_targets(&target))
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
                        .parser
                        .find_enclosing_context(self.call_node)
                        .class_name
                        .into_iter()
                        .flat_map(|class_name| {
                            self.field_type_candidates(&class_name, &property_name)
                        })
                        .collect();
                }

                self.visit(object_node)
                    .into_iter()
                    .flat_map(|class_name| self.field_type_candidates(&class_name, &property_name))
                    .collect()
            }
            "parenthesized_expression" => (0..expression_node.named_child_count())
                .filter_map(|i| expression_node.named_child(i as u32))
                .flat_map(|child| self.visit(child))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn resolve_expression_targets(mut self, expression_node: Node<'a>) -> Vec<String> {
        let mut targets = self.visit(expression_node);
        targets.sort_unstable();
        targets.dedup();
        targets
    }

    fn resolve_symbolic_class_targets(&self, target: &str) -> Vec<String> {
        if target.is_empty() {
            return Vec::new();
        }
        if self.facts.class_names.contains(target) {
            return vec![target.to_string()];
        }
        if target == "this" {
            return self
                .parser
                .find_enclosing_context(self.call_node)
                .class_name
                .into_iter()
                .collect();
        }
        if let Some(chain) = target.strip_prefix("this.") {
            return self
                .parser
                .find_enclosing_context(self.call_node)
                .class_name
                .into_iter()
                .flat_map(|class_name| self.resolve_field_chain(&class_name, chain))
                .collect();
        }
        Vec::new()
    }

    fn resolve_field_chain(&self, root_class: &str, chain: &str) -> Vec<String> {
        let mut current = vec![root_class.to_string()];
        for segment in chain.split('.') {
            if segment.is_empty() {
                return Vec::new();
            }
            let mut next = Vec::new();
            for class_name in &current {
                next.extend(self.field_type_candidates(class_name, segment));
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

    fn field_type_candidates(&self, class_name: &str, field_name: &str) -> Vec<String> {
        let mut targets: Vec<String> = self
            .facts
            .field_types_by_class
            .get(class_name)
            .and_then(|fields| fields.get(field_name))
            .cloned()
            .into_iter()
            .flatten()
            .flat_map(|field_type| {
                field_type
                    .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .filter(|token| !token.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
                    .into_iter()
            })
            .filter(|candidate| self.facts.class_names.contains(candidate))
            .collect();
        targets.sort_unstable();
        targets.dedup();
        targets
    }
}

pub(crate) fn resolve_call_targets_for_identifier(
    parser: &ParseContext,
    function_node: Node<'_>,
    call_node: Node<'_>,
    identifier_name: &str,
) -> Vec<String> {
    JsAliasResolver::new(parser, function_node, call_node)
        .resolve_identifier_targets(identifier_name)
}

pub(crate) fn type_facts(parser: &ParseContext) -> Rc<JsTypeFacts> {
    if let Some(cached) = parser.caches.borrow().language.js.type_facts.as_ref() {
        return Rc::clone(cached);
    }

    let facts = Rc::new(JsTypeFactsBuilder::new(parser).build());
    parser.caches.borrow_mut().language.js.type_facts = Some(Rc::clone(&facts));
    facts
}

fn anonymous_function_name(context: &ParseContext, function_node: Node<'_>) -> Option<String> {
    let parent = function_node.parent()?;
    match parent.kind() {
        "variable_declarator" => {
            let name_node = parent.child_by_field_name("name")?;
            if name_node.kind() == "identifier" {
                return Some(context.node_text(name_node));
            }
        }
        "assignment_expression" | "assignment" => {
            let left_node = parent.child_by_field_name("left")?;
            if left_node.kind() == "identifier" {
                return Some(context.node_text(left_node));
            }
        }
        "pair" | "property" => {
            let key_node = parent.child_by_field_name("key")?;
            if matches!(
                key_node.kind(),
                "identifier" | "property_identifier" | "string"
            ) {
                let text = context.node_text_lossy(key_node);
                return Some(text.trim_matches(|c| c == '"' || c == '\'').to_string());
            }
        }
        "export_statement" => {
            for i in 0..parent.child_count() {
                let child = parent.child(i as u32).unwrap();
                if context.node_text_eq(child, "default") {
                    return Some("<default_export>".to_string());
                }
            }
        }
        _ => {}
    }
    None
}

pub(crate) fn split_attribute_parts(
    parser: &ParseContext,
    node: Node<'_>,
) -> (String, Option<String>) {
    let mut callee = String::new();
    let mut obj_name: Option<String> = None;

    match node.kind() {
        "member_expression" => {
            if let Some(prop_node) = node.child_by_field_name("property") {
                callee = parser.node_text(prop_node);
            }
            if let Some(obj_node) = node.child_by_field_name("object") {
                obj_name = Some(parser.node_text(obj_node));
            }
        }
        _ => {
            let mut ids = Vec::new();
            for i in 0..node.named_child_count() {
                let child = node.named_child(i as u32).unwrap();
                if matches!(
                    child.kind(),
                    "identifier"
                        | "property_identifier"
                        | "private_property_identifier"
                        | "field_identifier"
                ) {
                    ids.push(parser.node_text(child));
                } else if child.kind() == "member_expression" {
                    obj_name = Some(parser.node_text(child));
                }
            }
            if let Some(last) = ids.last() {
                callee = last.clone();
                if ids.len() > 1 && obj_name.is_none() {
                    obj_name = Some(ids[0].clone());
                }
            }
        }
    }

    (callee, obj_name)
}

fn resolve_call_object_name(
    parser: &ParseContext,
    call_node: Node<'_>,
    object_node: Node<'_>,
) -> Option<String> {
    let node_id = parser.node_id(object_node);
    if let Some(cached) = parser
        .caches
        .borrow()
        .language
        .js
        .member_call_object_resolution
        .get(&node_id)
        .cloned()
    {
        return cached;
    }
    let facts = type_facts(parser);
    let mut targets = ExpressionTargetResolver::new(parser, call_node, &facts)
        .resolve_expression_targets(object_node);
    targets.sort_unstable();
    targets.dedup();
    let resolved = (targets.len() == 1).then(|| targets.swap_remove(0));
    parser
        .caches
        .borrow_mut()
        .language
        .js
        .member_call_object_resolution
        .insert(node_id, resolved.clone());
    resolved
}

fn collect_super_types_from_expr(
    parser: &ParseContext,
    node: Node<'_>,
    super_classes: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    match node.kind() {
        "identifier" | "type_identifier" | "property_identifier" => {
            let name = parser.node_text(node).trim().to_string();
            if !name.is_empty() && seen.insert(name.clone()) {
                super_classes.push(name);
            }
        }
        "member_expression" => {
            let text = parser.node_text(node).trim().to_string();
            if !text.is_empty() && seen.insert(text.clone()) {
                super_classes.push(text);
            }
            for i in (0..node.child_count()).rev() {
                let child = node.child(i as u32).unwrap();
                if matches!(child.kind(), "identifier" | "property_identifier") {
                    let name = parser.node_text(child).trim().to_string();
                    if !name.is_empty() && seen.insert(name.clone()) {
                        super_classes.push(name);
                    }
                    break;
                }
            }
        }
        "expression_with_type_arguments" => {
            if let Some(expr) = node.child_by_field_name("expression") {
                collect_super_types_from_expr(parser, expr, super_classes, seen);
                return;
            }
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i as u32) {
                    collect_super_types_from_expr(parser, child, super_classes, seen);
                    return;
                }
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                collect_super_types_from_expr(parser, function, super_classes, seen);
            }
        }
        _ => {
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i as u32) {
                    collect_super_types_from_expr(parser, child, super_classes, seen);
                }
            }
        }
    }
}
