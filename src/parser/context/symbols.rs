use std::collections::HashSet;

use tree_sitter::Node;

use crate::languages::QueryKind;
use crate::models::{
    AnnotationInfo, CallInfo, FieldInfo, FunctionParamInfo, ImportInfo, PythonPropertyCallerInfo,
    PythonPropertyInfo, RefInfo, RefKey,
};
use crate::parser::capture;
use crate::parser::languages::python;

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
        let mut matches = capture::collect_call_capture_matches(self);
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
        capture::collect_import_capture_matches(self, QueryKind::Import)
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

pub(crate) fn collect_class_fields(
    parser: &ParseContext,
    class_node: Node<'_>,
    class_name: &str,
    include_embedded_type_names: bool,
) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![class_node];

    while let Some(node) = stack.pop() {
        // Field collection also treats a bare JS `class` expression as a boundary.
        if node.id() != class_node.id()
            && (is_nested_class_boundary(node.kind()) || node.kind() == "class")
        {
            let nested_name = node
                .child_by_field_name("name")
                .map(|child| parser.node_text(child))
                .unwrap_or_default();
            if !nested_name.is_empty() && nested_name != class_name {
                continue;
            }
        }

        if matches!(
            node.kind(),
            "function_definition"
                | "method_definition"
                | "method_declaration"
                | "constructor_declaration"
        ) {
            continue;
        }

        if matches!(node.kind(), "field_definition" | "field_declaration") {
            let mut names = Vec::new();
            let mut field_type = node
                .child_by_field_name("type")
                .map(|child| parser.node_text(child));

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier"
                ) {
                    names.push(parser.node_text(child));
                } else if child.kind() == "variable_declarator" {
                    let mut child_cursor = child.walk();
                    for sub in child.children(&mut child_cursor) {
                        if sub.kind() == "identifier" {
                            names.push(parser.node_text(sub));
                            break;
                        }
                    }
                } else if field_type.is_none()
                    && matches!(
                        child.kind(),
                        "type_annotation"
                            | "type"
                            | "type_identifier"
                            | "integral_type"
                            | "floating_point_type"
                            | "boolean_type"
                            | "generic_type"
                            | "array_type"
                            | "scoped_type_identifier"
                    )
                {
                    field_type = Some(parser.node_text(child));
                }
            }

            if include_embedded_type_names && names.is_empty() {
                if let Some(field_type_text) = field_type.as_ref() {
                    let type_str = field_type_text.trim_start_matches('*');
                    let embedded_type_name = if type_str.contains('.') {
                        type_str.rsplit('.').next().unwrap_or(type_str)
                    } else {
                        type_str
                    };
                    names.push(embedded_type_name.to_string());
                }
            }

            for name in names {
                if !name.is_empty() && seen.insert(name.clone()) {
                    fields.push(FieldInfo {
                        name,
                        location: parser.node_location(node),
                        field_type: field_type.clone(),
                        class_name: Some(class_name.to_string()),
                    });
                }
            }
            continue;
        }

        push_children_reversed(&mut stack, node, false);
    }

    fields
}

pub(super) fn class_name(context: &ParseContext, node: Node<'_>) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let class_name = context.node_text(name_node);
        if !class_name.is_empty() {
            return Some(class_name);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "type_identifier" | "name") {
            let class_name = context.node_text(child);
            if !class_name.is_empty() {
                return Some(class_name);
            }
        }
    }
    None
}

pub(super) fn class_kind(context: &ParseContext, node: Node<'_>) -> String {
    match context.language() {
        "typescript" | "tsx" => match node.kind() {
            "abstract_class_declaration" => "abstract_class".to_string(),
            "interface_declaration" => "interface".to_string(),
            "type_alias_declaration" => "alias".to_string(),
            "enum_declaration" => "enum".to_string(),
            _ => "class".to_string(),
        },
        "java" => match node.kind() {
            "interface_declaration" => "interface".to_string(),
            "enum_declaration" => "enum".to_string(),
            "record_declaration" => "record".to_string(),
            "annotation_type_declaration" => "annotation".to_string(),
            _ => "class".to_string(),
        },
        "go" => node
            .children(&mut node.walk())
            .find(|child| child.kind() == "type_spec")
            .and_then(|type_spec| type_spec.child_by_field_name("type"))
            .map_or("struct", |type_node| match type_node.kind() {
                "interface_type" => "interface",
                _ => "struct",
            })
            .to_string(),
        _ => "class".to_string(),
    }
}

pub(super) fn class_methods(parser: &ParseContext, class_node: Node<'_>) -> Vec<String> {
    let mut methods = Vec::new();
    let mut stack = vec![class_node];
    while let Some(node) = stack.pop() {
        if node.id() != class_node.id() && is_nested_class_boundary(node.kind()) {
            continue;
        }
        if matches!(
            node.kind(),
            "function_definition"
                | "method_definition"
                | "method_declaration"
                | "constructor_declaration"
                | "method_elem"
                | "method_spec"
        ) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "identifier" | "property_identifier" | "field_identifier" | "name"
                ) {
                    methods.push(parser.node_text(child));
                    break;
                }
            }
            continue;
        }
        push_children_reversed(&mut stack, node, false);
    }
    methods
}

fn is_nested_class_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "class_definition"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration"
    )
}

/// Pushes `node`'s children onto `stack` in reverse so a stack-based DFS pops
/// them left-to-right. Set `named` to skip anonymous nodes.
fn push_children_reversed<'a>(stack: &mut Vec<Node<'a>>, node: Node<'a>, named: bool) {
    let mut cursor = node.walk();
    let children: Vec<_> = if named {
        node.named_children(&mut cursor).collect()
    } else {
        node.children(&mut cursor).collect()
    };
    for child in children.into_iter().rev() {
        stack.push(child);
    }
}
