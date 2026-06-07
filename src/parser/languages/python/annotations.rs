use tree_sitter::Node;

use crate::models::AnnotationInfo;
use crate::parser::ParseContext;

pub(super) struct PythonAnnotationCollector<'a> {
    parser: &'a ParseContext,
}

impl<'a> PythonAnnotationCollector<'a> {
    pub(super) fn new(parser: &'a ParseContext) -> Self {
        Self { parser }
    }

    pub(super) fn collect(&self) -> Vec<AnnotationInfo> {
        let mut annotations = Vec::new();
        let mut stack = vec![self.parser.tree().root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "decorated_definition" {
                self.collect_from_decorated(node, &mut annotations);
                let mut cursor = node.walk();
                let children: Vec<_> = node.children(&mut cursor).collect();
                for child in children.into_iter().rev() {
                    stack.push(child);
                }
                continue;
            }
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
        annotations
    }

    fn collect_from_decorated(
        &self,
        decorated_node: Node<'_>,
        annotations: &mut Vec<AnnotationInfo>,
    ) {
        let definition = decorated_node.child_by_field_name("definition");
        let (target_name, target_type, target_signature) = match definition {
            Some(definition_node) => {
                let name = definition_node
                    .child_by_field_name("name")
                    .map(|node| self.parser.node_text(node))
                    .unwrap_or_default();
                let target_kind = match definition_node.kind() {
                    "function_definition" => {
                        if self
                            .parser
                            .find_enclosing_context(definition_node)
                            .class_name
                            .is_some()
                        {
                            "method"
                        } else {
                            "function"
                        }
                    }
                    "class_definition" => "class",
                    _ => "",
                };
                (
                    name,
                    target_kind.to_string(),
                    self.definition_header(definition_node),
                )
            }
            None => (String::new(), String::new(), String::new()),
        };

        let mut cursor = decorated_node.walk();
        for child in decorated_node.children(&mut cursor) {
            if child.kind() != "decorator" {
                continue;
            }

            let name = self.decorator_name(child);
            if name.is_empty() {
                continue;
            }

            annotations.push(AnnotationInfo {
                name,
                signature: self.parser.node_text(child),
                location: self.parser.node_location(child),
                target_name: target_name.clone(),
                target_type: target_type.clone(),
                target_signature: format!("{}\n{}", self.parser.node_text(child), target_signature),
            });
        }
    }

    fn definition_header(&self, definition_node: Node<'_>) -> String {
        let end_byte = definition_node
            .child_by_field_name("body")
            .map_or_else(|| definition_node.end_byte(), |body| body.start_byte());
        self.parser
            .source_slice(definition_node.start_byte(), end_byte)
            .trim_end()
            .to_string()
    }

    fn decorator_name(&self, decorator_node: Node<'_>) -> String {
        let mut cursor = decorator_node.walk();
        for child in decorator_node.children(&mut cursor) {
            match child.kind() {
                "identifier" | "attribute" => return self.parser.node_text(child),
                "call" => {
                    return child
                        .child_by_field_name("function")
                        .map(|node| self.parser.node_text(node))
                        .unwrap_or_default();
                }
                _ => {}
            }
        }
        String::new()
    }
}
