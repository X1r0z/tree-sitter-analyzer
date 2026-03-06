use tree_sitter::Node;

use super::BaseParser;
use crate::nodes::FieldInfo;

#[allow(dead_code)]
impl BaseParser {
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
