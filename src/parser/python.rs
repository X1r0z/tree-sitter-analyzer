use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::{BaseParser, PythonPropertyCallers, PythonPropertyDefinitions};
use crate::nodes::FieldInfo;

#[allow(dead_code)]
impl BaseParser {
    pub(crate) fn build_python_property_indexes(
        &self,
    ) -> (PythonPropertyDefinitions, PythonPropertyCallers) {
        if self.language != "python" {
            return (HashSet::new(), HashMap::new());
        }

        let mut properties = HashSet::new();
        let mut callers_by_property: HashMap<String, Vec<(String, usize)>> = HashMap::new();
        let mut seen_callers: HashSet<(String, String, usize)> = HashSet::new();
        let mut stack = vec![self.tree.root_node()];

        while let Some(node) = stack.pop() {
            match node.kind() {
                "decorated_definition" => {
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
                    for i in 0..node.named_child_count() {
                        let Some(child) = node.named_child(i as u32) else {
                            continue;
                        };
                        if child.kind() == "decorator"
                            && self.node_trimmed_eq_str(child, "@property")
                        {
                            is_property = true;
                            break;
                        }
                    }

                    if is_property {
                        properties.insert((
                            self.node_text(name_node),
                            self.find_enclosing_class_name(definition_node),
                        ));
                    }
                }
                "attribute" => {
                    let Some(property_name) = self.extract_attribute_callee_name(node) else {
                        continue;
                    };
                    let caller = self
                        .find_enclosing_function_name(node)
                        .unwrap_or_else(|| "<module>".to_string());
                    let line = node.start_position().row + 1;
                    let seen_key = (property_name.clone(), caller.clone(), line);
                    if seen_callers.insert(seen_key) {
                        callers_by_property
                            .entry(property_name)
                            .or_default()
                            .push((caller, line));
                    }
                }
                _ => {}
            }

            for i in (0..node.named_child_count()).rev() {
                if let Some(child) = node.named_child(i as u32) {
                    stack.push(child);
                }
            }
        }

        (properties, callers_by_property)
    }

    pub(crate) fn is_python_property(&self, function_name: &str, class_name: Option<&str>) -> bool {
        let (properties, _) = self.build_python_property_indexes();
        properties.contains(&(function_name.to_string(), class_name.map(str::to_string)))
    }

    pub(crate) fn find_callers_of_python_property(
        &self,
        property_name: &str,
    ) -> Vec<(String, usize)> {
        let (_, callers_by_property) = self.build_python_property_indexes();
        callers_by_property
            .get(property_name)
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn extract_python_field_infos(
        &self,
        class_node: Node,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        let fields: Vec<FieldInfo> = Vec::new();
        let seen: HashSet<String> = HashSet::new();

        struct WalkCtx<'a> {
            analyzer: &'a BaseParser,
            class_node_id: usize,
            class_name: String,
            fields: Vec<FieldInfo>,
            seen: HashSet<String>,
        }

        fn collect_field_infos(ctx: &mut WalkCtx, node: Node, inside_method: bool) {
            if node.id() != ctx.class_node_id
                && matches!(
                    node.kind(),
                    "class_definition"
                        | "class_declaration"
                        | "class"
                        | "interface_declaration"
                        | "enum_declaration"
                        | "record_declaration"
                        | "annotation_type_declaration"
                )
            {
                let nested_name = node
                    .child_by_field_name("name")
                    .map(|n| ctx.analyzer.node_text(n))
                    .unwrap_or_default();
                if !nested_name.is_empty() && nested_name != ctx.class_name {
                    return;
                }
            }

            if matches!(
                node.kind(),
                "function_definition"
                    | "method_definition"
                    | "method_declaration"
                    | "constructor_declaration"
            ) {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i as u32) {
                        collect_field_infos(ctx, child, true);
                    }
                }
                return;
            }

            if node.kind() == "expression_statement" {
                for i in 0..node.child_count() {
                    let child = node.child(i as u32).unwrap();
                    if child.kind() != "assignment" {
                        continue;
                    }

                    let left_node = match child.child_by_field_name("left") {
                        Some(n) => n,
                        None => continue,
                    };
                    let field_type = child
                        .child_by_field_name("type")
                        .map(|n| ctx.analyzer.node_text(n));
                    let mut name = String::new();

                    if inside_method {
                        if left_node.kind() == "attribute" {
                            let obj_node = left_node.child_by_field_name("object");
                            let attr_node = left_node.child_by_field_name("attribute");
                            if let (Some(obj), Some(attr)) = (obj_node, attr_node) {
                                if ctx.analyzer.node_eq_str(obj, "self") {
                                    name = ctx.analyzer.node_text(attr);
                                }
                            }
                        }
                    } else if left_node.kind() == "identifier" {
                        name = ctx.analyzer.node_text(left_node);
                    }

                    if !name.is_empty() && ctx.seen.insert(name.clone()) {
                        ctx.fields.push(FieldInfo {
                            name,
                            location: ctx.analyzer.node_location(child),
                            field_type,
                            class_name: Some(ctx.class_name.clone()),
                        });
                    }
                }
                return;
            }

            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    collect_field_infos(ctx, child, inside_method);
                }
            }
        }

        let mut ctx = WalkCtx {
            analyzer: self,
            class_node_id: class_node.id(),
            class_name: class_name.to_string(),
            fields,
            seen,
        };
        collect_field_infos(&mut ctx, class_node, false);
        ctx.fields
    }

    pub(super) fn extract_python_super_class_names(&self, class_node: Node) -> Vec<String> {
        let mut super_classes = Vec::new();
        for i in 0..class_node.child_count() {
            let child = class_node.child(i as u32).unwrap();
            if child.kind() != "argument_list" {
                continue;
            }
            for j in 0..child.child_count() {
                let arg = child.child(j as u32).unwrap();
                if matches!(arg.kind(), "identifier" | "attribute") {
                    super_classes.push(self.node_text(arg));
                }
            }
        }
        super_classes
    }
}
