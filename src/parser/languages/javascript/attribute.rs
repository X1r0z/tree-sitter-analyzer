use tree_sitter::Node;

use crate::parser::ParseContext;

pub(super) struct AttributeParts {
    pub(super) name: String,
    pub(super) object: Option<String>,
}

impl AttributeParts {
    pub(super) fn from_member_expression(ctx: &ParseContext, node: Node<'_>) -> Self {
        let mut name = String::new();
        let mut object: Option<String> = None;

        if node.kind() == "member_expression" {
            if let Some(prop_node) = node.child_by_field_name("property") {
                name = ctx.node_text(prop_node);
            }
            if let Some(obj_node) = node.child_by_field_name("object") {
                object = Some(ctx.node_text(obj_node));
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
                    ids.push(ctx.node_text(child));
                } else if child.kind() == "member_expression" {
                    object = Some(ctx.node_text(child));
                }
            }
            if let Some(last) = ids.last() {
                name.clone_from(last);
                if ids.len() > 1 && object.is_none() {
                    object = Some(ids[0].clone());
                }
            }
        }

        Self { name, object }
    }
}
