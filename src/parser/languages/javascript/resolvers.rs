use std::collections::{HashMap, HashSet, VecDeque};

use tree_sitter::Node;

use crate::parser::ParseContext;

use super::binding_events::JsBindingEventCollector;
use super::index::receiver_index;
use super::semantic_facts::class_names;

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

    pub(crate) fn resolve<'a>(
        &'a mut self,
        call_start: usize,
        identifier_name: &'a str,
    ) -> Vec<String> {
        self.state.resolve(call_start, identifier_name)
    }

    pub(super) fn resolve_class_name(
        ctx: &ParseContext,
        function_node: Node<'_>,
        call_node: Node<'_>,
        object_node: Node<'_>,
    ) -> Option<String> {
        if !matches!(object_node.kind(), "identifier" | "property_identifier") {
            return None;
        }

        let object_name = ctx.node_text(object_node);
        if object_name.is_empty() {
            return None;
        }

        let class_names = class_names(ctx);
        if class_names.contains(&object_name) {
            return Some(object_name);
        }

        let mut class_targets =
            Self::resolve_targets_for_identifier(ctx, function_node, call_node, &object_name);
        (class_targets.len() == 1).then(|| class_targets.swap_remove(0))
    }

    fn resolve_targets_for_identifier(
        ctx: &ParseContext,
        function_node: Node<'_>,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        let function_id = ParseContext::node_id(function_node);
        let class_names = class_names(ctx);
        let symbolic_targets = {
            let mut caches = ctx.caches.borrow_mut();
            let resolver = caches
                .language
                .js
                .receiver_resolvers_by_function
                .entry(function_id)
                .or_insert_with(|| {
                    JsReceiverResolver::new(
                        JsBindingEventCollector::new(ctx)
                            .collect_receiver_events(function_node, class_names.as_ref()),
                    )
                });
            resolver.resolve(call_node.start_byte(), identifier_name)
        };

        let index = receiver_index(ctx);
        if symbolic_targets.len() == 1 {
            return index.symbolic_targets_via_index(ctx, call_node, &symbolic_targets[0]);
        }

        let mut resolved = symbolic_targets
            .into_iter()
            .flat_map(|target| index.symbolic_targets_via_index(ctx, call_node, &target))
            .collect::<Vec<_>>();
        resolved.sort_unstable();
        resolved.dedup();
        resolved
    }
}
