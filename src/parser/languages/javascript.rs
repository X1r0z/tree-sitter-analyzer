use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use tree_sitter::Node;

use super::super::capture::{self, CallCaptureMatch};
use super::super::{EnclosingContext, ParseContext};
use crate::languages::{LanguageEngine, QueryKind, ResolvedCall};
use crate::models::{FieldInfo, FunctionParamInfo};

pub(crate) struct JavaScriptFamilyEngine {
    language: &'static str,
}

#[derive(Clone)]
pub(crate) struct JsAliasEvent {
    pub(crate) start_byte: usize,
    pub(crate) name: String,
    pub(crate) targets: Vec<String>,
}

#[derive(Clone)]
struct ScopedBindingEvent {
    start_byte: usize,
    name: String,
    targets: Vec<String>,
}

struct ScopedEventResolverState {
    events: Vec<ScopedBindingEvent>,
    next_event_idx: usize,
    active_bindings: HashMap<String, Vec<String>>,
    last_call_start: usize,
}

#[derive(Clone)]
pub(crate) struct JsReceiverEvent {
    pub(crate) start_byte: usize,
    pub(crate) name: String,
    pub(crate) targets: Vec<String>,
}

pub(crate) struct JsAliasResolver {
    state: ScopedEventResolverState,
}

pub(crate) struct JsReceiverResolver {
    state: ScopedEventResolverState,
}

impl ScopedEventResolverState {
    fn new(events: Vec<ScopedBindingEvent>) -> Self {
        Self {
            events,
            next_event_idx: 0,
            active_bindings: HashMap::new(),
            last_call_start: 0,
        }
    }

    fn resolve<'a>(&'a mut self, call_start: usize, identifier_name: &'a str) -> Vec<String> {
        if call_start < self.last_call_start {
            self.next_event_idx = 0;
            self.active_bindings.clear();
        }
        self.last_call_start = call_start;

        while self.next_event_idx < self.events.len()
            && self.events[self.next_event_idx].start_byte < call_start
        {
            let event = &self.events[self.next_event_idx];
            self.active_bindings
                .insert(event.name.clone(), event.targets.clone());
            self.next_event_idx += 1;
        }

        let mut visited: HashSet<&'a str> = HashSet::new();
        let mut resolved = Vec::new();
        let mut resolved_seen: HashSet<&'a str> = HashSet::new();
        let mut queue: VecDeque<&'a str> = VecDeque::new();
        queue.push_back(identifier_name);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(targets) = self.active_bindings.get(current) {
                for target in targets {
                    queue.push_back(target.as_str());
                }
            } else if resolved_seen.insert(current) {
                resolved.push(current);
            }
        }

        resolved.sort_unstable();
        resolved.into_iter().map(str::to_string).collect()
    }
}

impl JsAliasResolver {
    pub(crate) fn new(events: Vec<JsAliasEvent>) -> Self {
        Self {
            state: ScopedEventResolverState::new(
                events
                    .into_iter()
                    .map(|event| ScopedBindingEvent {
                        start_byte: event.start_byte,
                        name: event.name,
                        targets: event.targets,
                    })
                    .collect(),
            ),
        }
    }

    pub(crate) fn resolve<'a>(
        &'a mut self,
        call_start: usize,
        identifier_name: &'a str,
    ) -> Vec<String> {
        self.state.resolve(call_start, identifier_name)
    }
}

impl JsReceiverResolver {
    pub(crate) fn new(events: Vec<JsReceiverEvent>) -> Self {
        Self {
            state: ScopedEventResolverState::new(
                events
                    .into_iter()
                    .map(|event| ScopedBindingEvent {
                        start_byte: event.start_byte,
                        name: event.name,
                        targets: event.targets,
                    })
                    .collect(),
            ),
        }
    }

    pub(crate) fn resolve_symbolic<'a>(
        &'a mut self,
        call_start: usize,
        identifier_name: &'a str,
    ) -> Vec<String> {
        self.state.resolve(call_start, identifier_name)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct JsSemanticFacts {
    pub(crate) class_names: HashSet<String>,
    pub(crate) constructor_arg_types: HashMap<(String, usize), Option<String>>,
    pub(crate) field_types_by_class: HashMap<String, HashMap<String, Option<String>>>,
    pub(crate) field_infos_by_class: HashMap<String, Vec<FieldInfo>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct JsTypeIndex {
    pub(crate) class_names: HashSet<String>,
    pub(crate) field_types_by_class: HashMap<String, HashMap<String, Option<String>>>,
    pub(crate) field_infos_by_class: HashMap<String, Vec<FieldInfo>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct JsReceiverIndex {
    pub(crate) class_names: HashSet<String>,
    pub(crate) field_targets_by_class: HashMap<String, HashMap<String, Vec<String>>>,
}

#[derive(Clone)]
struct PendingFieldInference {
    class_name: String,
    field_name: String,
    location: crate::models::Location,
    param_index: usize,
}

struct JsSemanticFactsBuilder<'a> {
    parser: &'a ParseContext,
    facts: JsSemanticFacts,
    pending_fields: Vec<PendingFieldInference>,
}

struct JsTypeHelper<'a> {
    parser: &'a ParseContext,
}

struct JsBindingEventCollector<'a> {
    parser: &'a ParseContext,
    type_helper: JsTypeHelper<'a>,
}

#[derive(Clone)]
struct TraversalContext<'a> {
    function_node: Option<Node<'a>>,
    class_name: Option<String>,
}

struct ExpressionTargetResolver<'a> {
    parser: &'a ParseContext,
    call_node: Node<'a>,
    function_node: Option<Node<'a>>,
    class_name: Option<String>,
    type_index: &'a JsTypeIndex,
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
    fn id(&self) -> &'static str {
        self.language
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
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
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
        let mut cursor = parameters.walk();
        for param in parameters.named_children(&mut cursor) {
            if let Some(info) = type_helper.build_param_info(param) {
                params.push(info);
            }
        }
        params
    }

    fn resolve_call<'a>(
        &self,
        ctx: &ParseContext,
        matched: &CallCaptureMatch<'a>,
        enclosing: &EnclosingContext<'a>,
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
                            object_name = resolved_object_name;
                            if let Some(function_node) = enclosing.function_node {
                                if let Some(object_node) = func_node.child_by_field_name("object") {
                                    if let Some(receiver_class_name) =
                                        JsReceiverResolver::resolve_class_name(
                                            ctx,
                                            function_node,
                                            call_node,
                                            object_node,
                                        )
                                    {
                                        object_name = Some(receiver_class_name);
                                    }
                                }
                            }
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

    fn resolve_call_targets(
        &self,
        ctx: &ParseContext,
        function_node: Node<'_>,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        let function_id = ParseContext::node_id(function_node);
        let mut caches = ctx.caches.borrow_mut();
        let resolver = caches
            .language
            .js
            .alias_resolvers_by_function
            .entry(function_id)
            .or_insert_with(|| {
                JsAliasResolver::new(
                    JsBindingEventCollector::new(ctx).collect_alias_events(function_node),
                )
            });
        resolver.resolve(call_node.start_byte(), identifier_name)
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
        type_index(ctx)
            .field_infos_by_class
            .get(class_name)
            .cloned()
            .unwrap_or_default()
    }

    fn super_classes(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
        let mut super_classes = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() != "class_heritage" {
                continue;
            }
            let mut child_cursor = child.walk();
            for sub in child.children(&mut child_cursor) {
                if matches!(sub.kind(), "extends_clause" | "implements_clause") {
                    let mut sub_cursor = sub.walk();
                    for grandchild in sub.children(&mut sub_cursor) {
                        if grandchild.is_named() {
                            collect_super_classes_from_expr(
                                ctx,
                                grandchild,
                                &mut super_classes,
                                &mut seen,
                            );
                        }
                    }
                } else if sub.is_named() {
                    collect_super_classes_from_expr(ctx, sub, &mut super_classes, &mut seen);
                }
            }
        }
        super_classes
    }
}

impl<'a> JsSemanticFactsBuilder<'a> {
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
            facts: JsSemanticFacts {
                class_names,
                ..JsSemanticFacts::default()
            },
            pending_fields: Vec::new(),
        }
    }

    fn build(mut self) -> JsSemanticFacts {
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
            if !self.parser.node_text_eq(name_node, "constructor") {
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
        let name = self.parser.node_text(name_node);
        if name.is_empty() {
            return;
        }
        let type_helper = self.type_helper();
        let field_type = member
            .child_by_field_name("type")
            .map(|type_node| JsTypeHelper::normalize_type_text(&self.parser.node_text(type_node)))
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
            let param_name = self.parser.node_text(pattern);
            if param_name.is_empty() {
                continue;
            }
            let param_type = param.child_by_field_name("type").map(|type_node| {
                JsTypeHelper::normalize_type_text(&self.parser.node_text(type_node))
            });
            self.upsert_field_fact(
                class_name,
                &param_name,
                self.parser.node_location(param),
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
            self.parser.tree().root_node(),
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
                                    self.parser,
                                    node,
                                    context.function_node,
                                    context.class_name.clone(),
                                    &type_index,
                                )
                                .resolve_expression_targets(argument);
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
                child_context.class_name = self.parser.cached_class_name(node);
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

impl JsReceiverResolver {
    fn resolve_class_name(
        parser: &ParseContext,
        function_node: Node<'_>,
        call_node: Node<'_>,
        object_node: Node<'_>,
    ) -> Option<String> {
        if !matches!(object_node.kind(), "identifier" | "property_identifier") {
            return None;
        }

        let object_name = parser.node_text(object_node);
        if object_name.is_empty() {
            return None;
        }

        let class_names = js_class_names(parser);
        if class_names.contains(&object_name) {
            return Some(object_name);
        }

        let mut class_targets =
            Self::resolve_targets_for_identifier(parser, function_node, call_node, &object_name);
        (class_targets.len() == 1).then(|| class_targets.swap_remove(0))
    }

    fn resolve_targets_for_identifier(
        parser: &ParseContext,
        function_node: Node<'_>,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        let function_id = ParseContext::node_id(function_node);
        let class_names = js_class_names(parser);
        let symbolic_targets = {
            let mut caches = parser.caches.borrow_mut();
            let resolver = caches
                .language
                .js
                .receiver_resolvers_by_function
                .entry(function_id)
                .or_insert_with(|| {
                    JsReceiverResolver::new(
                        JsBindingEventCollector::new(parser)
                            .collect_receiver_events(function_node, class_names.as_ref()),
                    )
                });
            resolver.resolve_symbolic(call_node.start_byte(), identifier_name)
        };

        let index = receiver_index(parser);
        if symbolic_targets.len() == 1 {
            return index.resolve_symbolic_targets(parser, call_node, &symbolic_targets[0]);
        }

        let mut resolved = symbolic_targets
            .into_iter()
            .flat_map(|target| index.resolve_symbolic_targets(parser, call_node, &target))
            .collect::<Vec<_>>();
        resolved.sort_unstable();
        resolved.dedup();
        resolved
    }
}

impl<'a> JsTypeHelper<'a> {
    fn new(parser: &'a ParseContext) -> Self {
        Self { parser }
    }

    fn extract_class_targets(class_names: &HashSet<String>, field_type: &str) -> Vec<String> {
        let mut targets: Vec<String> = field_type
            .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
            .filter(|token| !token.is_empty())
            .filter(|candidate| class_names.contains(*candidate))
            .map(str::to_string)
            .collect();
        targets.sort_unstable();
        targets.dedup();
        targets
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
        let mut cursor = member.walk();
        for child in member.named_children(&mut cursor) {
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
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
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

    fn normalize_type_text(text: &str) -> String {
        let stripped = text.trim();
        if let Some(rest) = stripped.strip_prefix(':') {
            rest.trim().to_string()
        } else {
            stripped.to_string()
        }
    }

    fn build_param_info(&self, param: Node<'_>) -> Option<FunctionParamInfo> {
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

    fn find_param_name_node(node: Node<'_>) -> Option<Node<'_>> {
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
            return Some(Self::normalize_type_text(&self.parser.node_text(type_node)));
        }

        node.child_by_field_name("pattern")
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| node.child_by_field_name("left"))
            .and_then(|child| self.find_param_type(child))
    }

    fn collect_constructor_params(&self, params_node: Node<'_>) -> Vec<FunctionParamInfo> {
        let mut params = Vec::new();
        let mut cursor = params_node.walk();
        for param in params_node.named_children(&mut cursor) {
            if let Some(info) = self.build_param_info(param) {
                params.push(info);
            }
        }
        params
    }
}

impl<'a> JsBindingEventCollector<'a> {
    fn new(parser: &'a ParseContext) -> Self {
        Self {
            parser,
            type_helper: JsTypeHelper::new(parser),
        }
    }

    fn collect_alias_events(&self, function_node: Node<'a>) -> Vec<JsAliasEvent> {
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

    fn collect_receiver_events(
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

impl<'a> ExpressionTargetResolver<'a> {
    fn type_helper(&self) -> JsTypeHelper<'a> {
        JsTypeHelper::new(self.parser)
    }

    fn new(
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

    fn resolve_expression_targets(mut self, expression_node: Node<'a>) -> Vec<String> {
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

impl JsReceiverIndex {
    fn resolve_symbolic_targets(
        &self,
        parser: &ParseContext,
        call_node: Node<'_>,
        target: &str,
    ) -> Vec<String> {
        if target.is_empty() {
            return Vec::new();
        }
        if self.class_names.contains(target) {
            return vec![target.to_string()];
        }
        if target == "this" {
            return parser
                .find_enclosing_context(call_node)
                .class_name
                .into_iter()
                .collect();
        }
        if let Some(chain) = target.strip_prefix("this.") {
            return parser
                .find_enclosing_context(call_node)
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
                next.extend(self.targets_for_field(class_name, segment));
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

    fn targets_for_field(&self, class_name: &str, field_name: &str) -> Vec<String> {
        self.field_targets_by_class
            .get(class_name)
            .and_then(|fields| fields.get(field_name))
            .cloned()
            .unwrap_or_default()
    }
}

fn js_class_names(parser: &ParseContext) -> Rc<HashSet<String>> {
    if let Some(cached) = parser.caches.borrow().language.js.class_names.as_ref() {
        return Rc::clone(cached);
    }

    let class_names = Rc::new(semantic_facts(parser).class_names.clone());
    parser.caches.borrow_mut().language.js.class_names = Some(Rc::clone(&class_names));
    class_names
}

pub(crate) fn semantic_facts(parser: &ParseContext) -> Rc<JsSemanticFacts> {
    if let Some(cached) = parser.caches.borrow().language.js.semantic_facts.as_ref() {
        return Rc::clone(cached);
    }

    let facts = Rc::new(JsSemanticFactsBuilder::new(parser).build());
    parser.caches.borrow_mut().language.js.semantic_facts = Some(Rc::clone(&facts));
    facts
}

pub(crate) fn type_index(parser: &ParseContext) -> Rc<JsTypeIndex> {
    if let Some(cached) = parser.caches.borrow().language.js.type_index.as_ref() {
        return Rc::clone(cached);
    }

    let semantic = semantic_facts(parser);
    let index = Rc::new(JsTypeIndex {
        class_names: semantic.class_names.clone(),
        field_types_by_class: semantic.field_types_by_class.clone(),
        field_infos_by_class: semantic.field_infos_by_class.clone(),
    });
    parser.caches.borrow_mut().language.js.type_index = Some(Rc::clone(&index));
    index
}

pub(crate) fn receiver_index(parser: &ParseContext) -> Rc<JsReceiverIndex> {
    if let Some(cached) = { parser.caches.borrow().language.js.receiver_index.clone() } {
        return cached;
    }

    let semantic = semantic_facts(parser);
    let mut field_targets_by_class = HashMap::new();
    for (class_name, fields) in &semantic.field_types_by_class {
        let mut targets_by_field = HashMap::new();
        for (field_name, field_type) in fields {
            let targets = field_type
                .as_deref()
                .map(|field_type| {
                    JsTypeHelper::extract_class_targets(&semantic.class_names, field_type)
                })
                .unwrap_or_default();
            if !targets.is_empty() {
                targets_by_field.insert(field_name.clone(), targets);
            }
        }
        if !targets_by_field.is_empty() {
            field_targets_by_class.insert(class_name.clone(), targets_by_field);
        }
    }

    let index = Rc::new(JsReceiverIndex {
        class_names: semantic.class_names.clone(),
        field_targets_by_class,
    });
    parser.caches.borrow_mut().language.js.receiver_index = Some(Rc::clone(&index));
    index
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
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
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
    let mut object_name: Option<String> = None;

    if node.kind() == "member_expression" {
        if let Some(prop_node) = node.child_by_field_name("property") {
            callee = parser.node_text(prop_node);
        }
        if let Some(obj_node) = node.child_by_field_name("object") {
            object_name = Some(parser.node_text(obj_node));
        }
    } else {
        let mut ids = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if matches!(
                child.kind(),
                "identifier"
                    | "property_identifier"
                    | "private_property_identifier"
                    | "field_identifier"
            ) {
                ids.push(parser.node_text(child));
            } else if child.kind() == "member_expression" {
                object_name = Some(parser.node_text(child));
            }
        }
        if let Some(last) = ids.last() {
            callee.clone_from(last);
            if ids.len() > 1 && object_name.is_none() {
                object_name = Some(ids[0].clone());
            }
        }
    }

    (callee, object_name)
}

fn collect_super_classes_from_expr(
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
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
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
                collect_super_classes_from_expr(parser, expr, super_classes, seen);
                return;
            }
            let mut cursor = node.walk();
            let first_child = node.named_children(&mut cursor).next();
            if let Some(child) = first_child {
                collect_super_classes_from_expr(parser, child, super_classes, seen);
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                collect_super_classes_from_expr(parser, function, super_classes, seen);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_super_classes_from_expr(parser, child, super_classes, seen);
            }
        }
    }
}
