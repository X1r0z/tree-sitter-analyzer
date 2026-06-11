use std::collections::HashSet;

use tree_sitter::Node;

use crate::languages::QueryKind;
use crate::models::{
    AnnotationInfo, CallInfo, FunctionParamInfo, ImportInfo, PythonPropertyCallerInfo,
    PythonPropertyInfo, RefInfo, RefKey,
};
use crate::parser::capture;
use crate::parser::languages::python;

use super::class_info::push_children_reversed;
use super::enclosing::{EnclosingContext, EnclosingIntervalLookup};
use super::ParseContext;

impl ParseContext {
    pub(crate) fn collect_properties(&self) -> Vec<PythonPropertyInfo> {
        python::PythonPropertyAnalyzer::new(self).collect_properties()
    }

    pub(crate) fn collect_property_callers(
        &self,
        property_name: Option<&str>,
    ) -> Vec<PythonPropertyCallerInfo> {
        python::PythonPropertyAnalyzer::new(self).collect_callers(property_name)
    }

    pub(crate) fn collect_function_params(&self, function_node: Node) -> Vec<FunctionParamInfo> {
        let target = self.engine().normalize_function_node(self, function_node);
        self.engine().function_params(self, target)
    }

    pub(crate) fn collect_calls(&self) -> Vec<CallInfo> {
        let mut matches = capture::collect_call_matches(self);
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
                    .map(|lookup| lookup.enclosing_at(call_node))
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
                && callee_function_node.is_some_and(|node| node.kind() == "identifier")
            {
                if let Some(function_node) = enclosing.function_node {
                    let resolved =
                        self.resolve_call_targets(function_node, call_node, &callee);
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
        capture::collect_import_matches(self, QueryKind::Import)
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

        let expected: HashSet<RefKey> = candidates.iter().map(RefKey::from).collect();
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
                push_children_reversed(&mut stack, node, true);
            }
        }
        refs
    }
}
