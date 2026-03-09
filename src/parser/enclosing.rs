use tree_sitter::Node;

use super::languages::go;
use super::ParseContext;

pub(crate) struct EnclosingContext<'a> {
    pub(crate) function_name: Option<String>,
    pub(crate) class_name: Option<String>,
    pub(crate) function_node: Option<Node<'a>>,
}

impl ParseContext {
    pub(crate) fn find_enclosing_context<'a>(&self, node: Node<'a>) -> EnclosingContext<'a> {
        if let Some(class_name) = find_language_specific_enclosing_class_name(self, node) {
            return EnclosingContext {
                function_name: self.cached_function_name(node),
                class_name: Some(class_name),
                function_node: Some(node),
            };
        }

        let mut current = node.parent();
        let mut function_name: Option<String> = None;
        let mut class_name: Option<String> = None;
        let mut function_node: Option<Node<'a>> = None;

        while let Some(cur) = current {
            if function_name.is_none() && is_function_like(cur.kind()) {
                function_name = self.cached_function_name(cur);
                function_node = Some(cur);
            }

            if class_name.is_none() {
                class_name = find_language_specific_enclosing_class_name(self, cur);
            }

            if class_name.is_none()
                && matches!(
                    cur.kind(),
                    "class_definition"
                        | "class_declaration"
                        | "class_body"
                        | "interface_declaration"
                        | "enum_declaration"
                        | "record_declaration"
                        | "annotation_type_declaration"
                )
            {
                class_name = self.cached_class_name(cur);
                current = cur.parent();
                continue;
            }

            current = cur.parent();
        }

        EnclosingContext {
            function_name,
            class_name,
            function_node,
        }
    }

    pub(crate) fn cached_function_name(&self, node: Node) -> Option<String> {
        let node_id = node.id();
        if let Some(name) = self.function_names_by_node.borrow().get(&node_id) {
            return name.clone();
        }
        let name = function_name_from_node(self, node);
        self.function_names_by_node
            .borrow_mut()
            .insert(node_id, name.clone());
        name
    }

    pub(crate) fn cached_class_name(&self, node: Node) -> Option<String> {
        let node_id = node.id();
        if let Some(name) = self.class_names_by_node.borrow().get(&node_id) {
            return name.clone();
        }
        let name = class_name_from_node(self, node);
        self.class_names_by_node
            .borrow_mut()
            .insert(node_id, name.clone());
        name
    }
}

fn find_language_specific_enclosing_class_name(
    parser: &ParseContext,
    node: Node<'_>,
) -> Option<String> {
    match parser.language.as_str() {
        "go" if node.kind() == "method_declaration" => go::go_receiver_type_name(parser, node),
        _ => None,
    }
}

fn is_function_like(node_kind: &str) -> bool {
    matches!(
        node_kind,
        "function_definition"
            | "async_function_definition"
            | "function_declaration"
            | "method_definition"
            | "arrow_function"
            | "method_declaration"
            | "constructor_declaration"
            | "function_expression"
            | "func_literal"
    )
}

fn anonymous_function_name(parser: &ParseContext, func_node: Node<'_>) -> Option<String> {
    let parent = func_node.parent()?;
    match parent.kind() {
        "variable_declarator" => {
            let name_node = parent.child_by_field_name("name")?;
            if name_node.kind() == "identifier" {
                return Some(parser.node_text(name_node));
            }
        }
        "assignment_expression" | "assignment" => {
            let left_node = parent.child_by_field_name("left")?;
            if left_node.kind() == "identifier" {
                return Some(parser.node_text(left_node));
            }
        }
        "pair" | "property" => {
            let key_node = parent.child_by_field_name("key")?;
            if matches!(
                key_node.kind(),
                "identifier" | "property_identifier" | "string"
            ) {
                let text = parser.node_text_lossy(key_node);
                return Some(text.trim_matches(|c| c == '"' || c == '\'').to_string());
            }
        }
        "export_statement" => {
            for i in 0..parent.child_count() {
                let child = parent.child(i as u32).unwrap();
                if parser.node_text_eq(child, "default") {
                    return Some("<default_export>".to_string());
                }
            }
        }
        _ => {}
    }
    None
}

fn function_name_from_node(parser: &ParseContext, node: Node<'_>) -> Option<String> {
    let anonymous_types = ["arrow_function", "func_literal"];
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(parser.node_text(name_node));
    }
    if anonymous_types.contains(&node.kind()) || node.kind() == "function_expression" {
        return anonymous_function_name(parser, node);
    }
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if matches!(
            child.kind(),
            "identifier" | "property_identifier" | "field_identifier"
        ) {
            return Some(parser.node_text(child));
        }
    }
    None
}

fn class_name_from_node(parser: &ParseContext, node: Node<'_>) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let class_name = parser.node_text(name_node);
        if !class_name.is_empty() {
            return Some(class_name);
        }
    }
    for i in 0..node.child_count() {
        let child = node.child(i as u32).unwrap();
        if matches!(child.kind(), "identifier" | "type_identifier" | "name") {
            let class_name = parser.node_text(child);
            if !class_name.is_empty() {
                return Some(class_name);
            }
        }
    }
    None
}
