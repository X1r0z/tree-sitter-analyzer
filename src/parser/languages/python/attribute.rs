use tree_sitter::Node;

use crate::parser::ParseContext;

pub(super) struct AttributeParts {
    pub(super) name: String,
    pub(super) object: Option<String>,
}

impl AttributeParts {
    pub(super) fn from_attribute(ctx: &ParseContext, node: Node<'_>) -> Self {
        let name = node
            .child_by_field_name("attribute")
            .map(|attr_node| ctx.node_text(attr_node))
            .unwrap_or_default();
        let object = node
            .child_by_field_name("object")
            .map(|obj_node| ctx.node_text(obj_node));

        Self { name, object }
    }
}
