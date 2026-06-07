use std::cell::Ref;
use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::attribute::AttributeParts;
use crate::models::{Location, PythonPropertyCallerInfo, PythonPropertyInfo};
use crate::parser::call_targets::{
    matches_property_target as call_matches_property_target, type_matches_class,
};
use crate::parser::{
    ParseContext, PythonPropertyCallers, PythonPropertyDefinitions, PythonPropertyIndexes,
};
use crate::traversal::collect_reachable_bfs;
use crate::utils::select_most_specific_by_line;

type PythonPropertyCallerKey = (String, String, Option<String>, Option<String>, usize, usize);

pub(crate) struct PythonPropertyAnalyzer<'a> {
    parser: &'a ParseContext,
}

impl<'a> PythonPropertyAnalyzer<'a> {
    pub(crate) fn new(parser: &'a ParseContext) -> Self {
        Self { parser }
    }

    pub(crate) fn collect_properties(&self) -> Vec<PythonPropertyInfo> {
        self.with_cached_definitions(|properties| {
            let mut values: Vec<_> = properties
                .iter()
                .cloned()
                .map(|(name, class_name)| PythonPropertyInfo { name, class_name })
                .collect();
            values.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then_with(|| left.class_name.cmp(&right.class_name))
            });
            values
        })
    }

    pub(crate) fn collect_callers(
        &self,
        property_name: Option<&str>,
    ) -> Vec<PythonPropertyCallerInfo> {
        self.with_cached_callers(|callers| {
            let mut values = Vec::new();

            match property_name {
                Some(property_name) => {
                    values.extend(callers.get(property_name).cloned().unwrap_or_default());
                }
                None => {
                    for entries in callers.values() {
                        values.extend(entries.iter().cloned());
                    }
                }
            }

            values.sort_by(|left, right| {
                left.property_name
                    .cmp(&right.property_name)
                    .then_with(|| left.caller.cmp(&right.caller))
                    .then_with(|| left.location.start_line.cmp(&right.location.start_line))
            });
            values
        })
    }

    fn ensure_definitions(&self) {
        if self.parser.language() != "python" {
            return;
        }
        if self
            .parser
            .caches
            .borrow()
            .language
            .python
            .property_indexes
            .is_some()
        {
            return;
        }
        let definitions = self.collect_definitions();
        self.parser
            .caches
            .borrow_mut()
            .language
            .python
            .property_indexes = Some(PythonPropertyIndexes {
            definitions,
            callers_by_property: None,
        });
    }

    fn ensure_callers(&self) {
        if self.parser.language() != "python" {
            return;
        }
        self.ensure_definitions();

        let should_build = self
            .parser
            .caches
            .borrow()
            .language
            .python
            .property_indexes
            .as_ref()
            .is_some_and(|indexes| indexes.callers_by_property.is_none());
        if !should_build {
            return;
        }

        let definitions = {
            let caches = self.parser.caches.borrow();
            caches
                .language
                .python
                .property_indexes
                .as_ref()
                .expect("python cache")
                .definitions
                .clone()
        };
        let candidate_callers = self.collect_candidate_callers();
        let callers_by_property = self.filter_callers(&definitions, &candidate_callers);
        if let Some(indexes) = self
            .parser
            .caches
            .borrow_mut()
            .language
            .python
            .property_indexes
            .as_mut()
        {
            indexes.callers_by_property = Some(callers_by_property);
        }
    }

    fn with_cached_definitions<R>(&self, f: impl FnOnce(&PythonPropertyDefinitions) -> R) -> R {
        if self.parser.language() != "python" {
            return f(&HashSet::new());
        }
        self.ensure_definitions();
        let caches = self.parser.caches.borrow();
        let definitions = Ref::map(caches, |caches| {
            &caches
                .language
                .python
                .property_indexes
                .as_ref()
                .expect("python cache")
                .definitions
        });
        f(&definitions)
    }

    fn with_cached_callers<R>(&self, f: impl FnOnce(&PythonPropertyCallers) -> R) -> R {
        if self.parser.language() != "python" {
            return f(&HashMap::new());
        }
        self.ensure_callers();
        let caches = self.parser.caches.borrow();
        let callers = Ref::map(caches, |caches| {
            caches
                .language
                .python
                .property_indexes
                .as_ref()
                .expect("python cache")
                .callers_by_property
                .as_ref()
                .expect("python callers cache")
        });
        f(&callers)
    }

    fn collect_definitions(&self) -> PythonPropertyDefinitions {
        if self.parser.language() != "python" {
            return HashSet::new();
        }

        let mut properties = HashSet::new();
        let mut stack = vec![self.parser.tree().root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "decorated_definition" {
                let Some(definition_node) = node.child_by_field_name("definition") else {
                    continue;
                };
                if definition_node.kind() != "function_definition" {
                    continue;
                }
                let Some(name_node) = definition_node.child_by_field_name("name") else {
                    continue;
                };

                let mut is_property = false;
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "decorator"
                        && self.parser.node_text_trimmed_eq(child, "@property")
                    {
                        is_property = true;
                        break;
                    }
                }

                if is_property {
                    properties.insert((
                        self.parser.node_text(name_node),
                        self.parser
                            .find_enclosing_context(definition_node)
                            .class_name,
                    ));
                }
            }

            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }

        properties
    }

    fn collect_candidate_callers(&self) -> PythonPropertyCallers {
        if self.parser.language() != "python" {
            return HashMap::new();
        }

        let module_binding_types = self.collect_module_binding_types();
        let mut callers_by_property: HashMap<String, Vec<PythonPropertyCallerInfo>> =
            HashMap::new();
        let mut seen_callers: HashSet<PythonPropertyCallerKey> = HashSet::new();
        let mut stack = vec![self.parser.tree().root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "attribute" {
                if !Self::is_load_like_property_access(node) {
                    continue;
                }
                let parts = AttributeParts::from_attribute(self.parser, node);
                if parts.name.is_empty() {
                    continue;
                }
                let enclosing = self.parser.find_enclosing_context(node);
                let caller = enclosing
                    .function_name
                    .unwrap_or_else(|| "<module>".to_string());
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;
                let object_type = if caller == "<module>" {
                    parts
                        .object
                        .as_deref()
                        .and_then(|name| module_binding_types.get(name))
                        .cloned()
                } else {
                    None
                };
                let seen_key = (
                    parts.name.clone(),
                    caller.clone(),
                    enclosing.class_name.clone(),
                    parts.object.clone(),
                    start_line,
                    end_line,
                );
                if seen_callers.insert(seen_key) {
                    callers_by_property
                        .entry(parts.name.clone())
                        .or_default()
                        .push(PythonPropertyCallerInfo {
                            location: Location {
                                file: self.parser.file_path().to_string_lossy().into_owned(),
                                start_line,
                                end_line,
                            },
                            property_name: parts.name.clone(),
                            caller,
                            caller_class_name: enclosing.class_name.clone(),
                            object_name: parts.object,
                            object_type,
                        });
                }
            }

            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }

        callers_by_property
    }

    fn is_load_like_property_access(node: Node<'_>) -> bool {
        let mut current = node;
        while let Some(parent) = current.parent() {
            match parent.kind() {
                "assignment" | "augmented_assignment"
                    if parent.child_by_field_name("left").is_some_and(|left| {
                        left.start_byte() <= node.start_byte() && node.end_byte() <= left.end_byte()
                    }) =>
                {
                    return false;
                }
                _ => {}
            }
            current = parent;
        }
        true
    }

    fn collect_module_binding_types(&self) -> HashMap<String, String> {
        let mut bindings = HashMap::new();
        let root = self.parser.tree().root_node();

        let mut cursor = root.walk();
        for node in root.named_children(&mut cursor) {
            let mut assignments = Vec::new();
            match node.kind() {
                "expression_statement" => {
                    let mut cursor = node.walk();
                    assignments.extend(
                        node.named_children(&mut cursor)
                            .filter(|child| child.kind() == "assignment"),
                    );
                }
                "assignment" => assignments.push(node),
                _ => {}
            }

            for assignment in assignments {
                let Some(left) = assignment.child_by_field_name("left") else {
                    continue;
                };
                if left.kind() != "identifier" {
                    continue;
                }
                let name = self.parser.node_text(left);
                if name.is_empty() {
                    continue;
                }

                if let Some(type_node) = assignment.child_by_field_name("type") {
                    let class_name = self
                        .parser
                        .node_text(type_node)
                        .rsplit('.')
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    if !class_name.is_empty() {
                        bindings.insert(name, class_name);
                        continue;
                    }
                }

                let Some(right) = assignment.child_by_field_name("right") else {
                    bindings.remove(&name);
                    continue;
                };
                if let Some(class_name) = self.infer_module_binding_type(right) {
                    bindings.insert(name, class_name);
                } else {
                    bindings.remove(&name);
                }
            }
        }

        bindings
    }

    fn infer_module_binding_type(&self, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "call" => node
                .child_by_field_name("function")
                .and_then(|function| self.infer_module_binding_type(function)),
            "identifier" | "attribute" => {
                let class_name = self
                    .parser
                    .node_text(node)
                    .rsplit('.')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                (!class_name.is_empty()).then_some(class_name)
            }
            _ => None,
        }
    }

    fn filter_callers(
        &self,
        property_definitions: &PythonPropertyDefinitions,
        callers: &PythonPropertyCallers,
    ) -> PythonPropertyCallers {
        let property_definitions: Vec<_> = property_definitions
            .iter()
            .cloned()
            .map(|(name, class_name)| PythonPropertyInfo { name, class_name })
            .collect();
        let parents_by_class = self
            .parser
            .collect_classes()
            .into_iter()
            .map(|class| (class.name, class.super_classes))
            .collect::<HashMap<_, _>>();
        let mut functions = self.parser.collect_functions(false);
        functions
            .sort_by_key(|function| (function.location.start_line, function.location.end_line));

        let mut filtered = HashMap::new();
        for (property_name, entries) in callers {
            let matched: Vec<_> = entries
                .iter()
                .filter(|caller| {
                    self.matches_known_property(
                        caller,
                        &property_definitions,
                        &parents_by_class,
                        &functions,
                    )
                })
                .cloned()
                .collect();
            if !matched.is_empty() {
                filtered.insert(property_name.clone(), matched);
            }
        }
        filtered
    }

    fn matches_known_property(
        &self,
        caller: &PythonPropertyCallerInfo,
        property_definitions: &[PythonPropertyInfo],
        parents_by_class: &HashMap<String, Vec<String>>,
        functions: &[crate::models::FunctionInfo],
    ) -> bool {
        let candidates: Vec<_> = property_definitions
            .iter()
            .filter(|property| property.name == caller.property_name)
            .collect();
        if candidates.is_empty() {
            return false;
        }

        if caller.caller == "<module>" {
            return candidates.iter().any(|property| {
                property.class_name.as_deref().is_some_and(|class_name| {
                    caller.object_name.as_deref() != Some(class_name)
                        && type_matches_class(caller.object_type.as_deref(), class_name)
                })
            });
        }

        let caller_params =
            select_most_specific_by_line(functions, caller.location.start_line, |function| {
                (function.location.start_line, function.location.end_line)
            })
            .filter(|function| {
                function.name == caller.caller
                    && function.class_name.as_deref() == caller.caller_class_name.as_deref()
            })
            .map(|function| function.params.clone())
            .unwrap_or_default();

        let caller_fields = caller
            .caller_class_name
            .as_deref()
            .map(|class_name| self.parser.collect_fields_for_class(class_name))
            .unwrap_or_default();

        candidates.iter().any(|property| {
            let Some(target_class_name) = property.class_name.as_deref() else {
                return false;
            };

            if matches!(caller.object_name.as_deref(), Some("self" | "cls")) {
                caller
                    .caller_class_name
                    .as_deref()
                    .is_some_and(|class_name| {
                        class_name == target_class_name
                            || collect_reachable_bfs(
                                [class_name.to_string()],
                                [class_name.to_string()],
                                |current| {
                                    parents_by_class.get(current).cloned().unwrap_or_default()
                                },
                                Clone::clone,
                                Clone::clone,
                            )
                            .into_iter()
                            .any(|ancestor| ancestor == target_class_name)
                    })
            } else {
                call_matches_property_target(
                    caller.caller_class_name.as_deref(),
                    caller.object_name.as_deref(),
                    target_class_name,
                    |attr_name, expected_class_name| {
                        caller_fields.iter().any(|field| {
                            field.name == attr_name
                                && type_matches_class(
                                    field.field_type.as_deref(),
                                    expected_class_name,
                                )
                        })
                    },
                    |attr_name, expected_class_name| {
                        caller_params.iter().any(|param| {
                            param.name == attr_name
                                && type_matches_class(
                                    param.param_type.as_deref(),
                                    expected_class_name,
                                )
                        })
                    },
                )
            }
        })
    }
}
