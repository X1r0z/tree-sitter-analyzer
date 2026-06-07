use std::collections::HashSet;

use tree_sitter::Node;

use crate::models::FieldInfo;
use crate::parser::ParseContext;

pub(super) struct PythonFieldCollector<'a> {
    parser: &'a ParseContext,
    class_node: Node<'a>,
    class_node_id: usize,
    class_name: String,
    fields: Vec<FieldInfo>,
    seen: HashSet<String>,
}

impl<'a> PythonFieldCollector<'a> {
    pub(super) fn new(parser: &'a ParseContext, class_node: Node<'a>, class_name: &str) -> Self {
        Self {
            parser,
            class_node,
            class_node_id: class_node.id(),
            class_name: class_name.to_string(),
            fields: Vec::new(),
            seen: HashSet::new(),
        }
    }

    pub(super) fn collect(mut self) -> Vec<FieldInfo> {
        self.walk(self.class_node, false);
        self.fields
    }

    fn walk(&mut self, node: Node<'a>, inside_method: bool) {
        if node.id() != self.class_node_id
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
                .map(|child| self.parser.node_text(child))
                .unwrap_or_default();
            if !nested_name.is_empty() && nested_name != self.class_name {
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
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk(child, true);
            }
            return;
        }

        if node.kind() == "expression_statement" {
            self.collect_assignment_fields(node, inside_method);
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, inside_method);
        }
    }

    fn collect_assignment_fields(&mut self, node: Node<'a>, inside_method: bool) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "assignment" {
                continue;
            }

            let Some(left_node) = child.child_by_field_name("left") else {
                continue;
            };
            let field_type = child
                .child_by_field_name("type")
                .map(|node| self.parser.node_text(node));
            let mut name = String::new();

            if inside_method {
                if left_node.kind() == "attribute" {
                    let obj_node = left_node.child_by_field_name("object");
                    let attr_node = left_node.child_by_field_name("attribute");
                    if let (Some(obj), Some(attr)) = (obj_node, attr_node) {
                        if self.parser.node_text_eq(obj, "self") {
                            name = self.parser.node_text(attr);
                        }
                    }
                }
            } else if left_node.kind() == "identifier" {
                name = self.parser.node_text(left_node);
            }

            if !name.is_empty() && self.seen.insert(name.clone()) {
                self.fields.push(FieldInfo {
                    name,
                    location: self.parser.node_location(child),
                    field_type,
                    class_name: Some(self.class_name.clone()),
                });
            }
        }
    }
}
