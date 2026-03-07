use tree_sitter::Node;

use super::BaseParser;
use crate::nodes::{AnnotationInfo, FieldInfo};

#[allow(dead_code)]
impl BaseParser {
    pub(super) fn extract_java_annotations(&self) -> Vec<AnnotationInfo> {
        let mut annotations = Vec::new();
        let mut stack = vec![self.tree.root_node()];

        while let Some(node) = stack.pop() {
            if matches!(node.kind(), "marker_annotation" | "annotation") {
                if let Some(info) = self.build_java_annotation_info(node) {
                    annotations.push(info);
                }
                continue;
            }
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
                }
            }
        }
        annotations
    }

    fn build_java_annotation_info(&self, node: Node) -> Option<AnnotationInfo> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(name_node);
        if name.is_empty() {
            return None;
        }

        let signature = self.node_text(node);
        let (target_name, target_type, target_signature) = self.find_java_annotation_target(node);

        Some(AnnotationInfo {
            name,
            signature,
            location: self.node_location(node),
            target_name,
            target_type,
            target_signature,
        })
    }

    fn find_java_annotation_target(&self, annotation_node: Node) -> (String, String, String) {
        let mut current = annotation_node;
        while let Some(parent) = current.parent() {
            let (target_name, target_type, target_signature) = match parent.kind() {
                "method_declaration" => (
                    parent
                        .child_by_field_name("name")
                        .map(|n| self.node_text(n))
                        .unwrap_or_default(),
                    "method".to_string(),
                    format!(
                        "{}\n{}",
                        self.node_text(annotation_node),
                        self.extract_java_signature(parent)
                    ),
                ),
                "constructor_declaration" => (
                    parent
                        .child_by_field_name("name")
                        .map(|n| self.node_text(n))
                        .unwrap_or_default(),
                    "constructor".to_string(),
                    format!(
                        "{}\n{}",
                        self.node_text(annotation_node),
                        self.extract_java_signature(parent)
                    ),
                ),
                "class_declaration" => (
                    parent
                        .child_by_field_name("name")
                        .map(|n| self.node_text(n))
                        .unwrap_or_default(),
                    "class".to_string(),
                    format!(
                        "{}\n{}",
                        self.node_text(annotation_node),
                        self.extract_java_class_header_line(parent)
                    ),
                ),
                "interface_declaration" => (
                    parent
                        .child_by_field_name("name")
                        .map(|n| self.node_text(n))
                        .unwrap_or_default(),
                    "interface".to_string(),
                    format!(
                        "{}\n{}",
                        self.node_text(annotation_node),
                        self.extract_java_class_header_line(parent)
                    ),
                ),
                "enum_declaration" => (
                    parent
                        .child_by_field_name("name")
                        .map(|n| self.node_text(n))
                        .unwrap_or_default(),
                    "enum".to_string(),
                    format!(
                        "{}\n{}",
                        self.node_text(annotation_node),
                        self.extract_java_class_header_line(parent)
                    ),
                ),
                "record_declaration" => (
                    parent
                        .child_by_field_name("name")
                        .map(|n| self.node_text(n))
                        .unwrap_or_default(),
                    "record".to_string(),
                    format!(
                        "{}\n{}",
                        self.node_text(annotation_node),
                        self.extract_java_class_header_line(parent)
                    ),
                ),
                "annotation_type_declaration" => (
                    parent
                        .child_by_field_name("name")
                        .map(|n| self.node_text(n))
                        .unwrap_or_default(),
                    "annotation_type".to_string(),
                    format!(
                        "{}\n{}",
                        self.node_text(annotation_node),
                        self.extract_java_class_header_line(parent)
                    ),
                ),
                "field_declaration" => {
                    let field_name = self.first_field_declarator_name(parent);
                    (field_name, "field".to_string(), self.node_text(parent))
                }
                "formal_parameter" | "spread_parameter" => (
                    parent
                        .child_by_field_name("name")
                        .map(|n| self.node_text(n))
                        .unwrap_or_default(),
                    "parameter".to_string(),
                    self.find_enclosing_java_callable_signature(parent),
                ),
                "local_variable_declaration" => {
                    let var_name = self.first_field_declarator_name(parent);
                    (var_name, "variable".to_string(), self.node_text(parent))
                }
                "modifiers" | "annotation_argument_list" => {
                    current = parent;
                    continue;
                }
                _ => {
                    current = parent;
                    continue;
                }
            };
            return (target_name, target_type, target_signature);
        }
        (String::new(), String::new(), String::new())
    }

    fn find_enclosing_java_callable_signature(&self, node: Node) -> String {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if matches!(
                parent.kind(),
                "method_declaration" | "constructor_declaration"
            ) {
                return self.extract_java_signature(parent);
            }
            current = parent;
        }
        String::new()
    }

    fn first_field_declarator_name(&self, decl_node: Node) -> String {
        for i in 0..decl_node.child_count() {
            let child = decl_node.child(i as u32).unwrap();
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    return self.node_text(name_node);
                }
            }
        }
        String::new()
    }

    pub(super) fn extract_java_field_infos(
        &self,
        class_node: Node,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        self.extract_declared_field_infos(class_node, class_name)
    }

    pub(super) fn extract_java_super_class_names(&self, class_node: Node) -> Vec<String> {
        let mut super_classes = Vec::new();

        for i in 0..class_node.child_count() {
            let child = class_node.child(i as u32).unwrap();
            match child.kind() {
                "superclass" => {
                    for j in 0..child.child_count() {
                        let sub = child.child(j as u32).unwrap();
                        if sub.kind() == "type_identifier" {
                            super_classes.push(self.node_text(sub));
                        } else if sub.kind() == "generic_type" {
                            for k in 0..sub.child_count() {
                                let g = sub.child(k as u32).unwrap();
                                if g.kind() == "type_identifier" {
                                    super_classes.push(self.node_text(g));
                                    break;
                                }
                            }
                        }
                    }
                }
                "super_interfaces" => {
                    for j in 0..child.child_count() {
                        let sub = child.child(j as u32).unwrap();
                        if sub.kind() != "type_list" {
                            continue;
                        }
                        for k in 0..sub.child_count() {
                            let t = sub.child(k as u32).unwrap();
                            if t.kind() == "type_identifier" {
                                super_classes.push(self.node_text(t));
                            } else if t.kind() == "generic_type" {
                                for l in 0..t.child_count() {
                                    let g = t.child(l as u32).unwrap();
                                    if g.kind() == "type_identifier" {
                                        super_classes.push(self.node_text(g));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        super_classes
    }
}
