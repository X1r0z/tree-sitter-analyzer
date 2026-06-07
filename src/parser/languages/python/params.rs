use tree_sitter::Node;

use crate::models::FunctionParamInfo;
use crate::parser::ParseContext;

pub(super) struct PythonParamHelper<'a> {
    ctx: &'a ParseContext,
}

impl<'a> PythonParamHelper<'a> {
    pub(super) fn new(ctx: &'a ParseContext) -> Self {
        Self { ctx }
    }

    pub(super) fn collect(&self, function_node: Node<'_>) -> Vec<FunctionParamInfo> {
        let Some(parameters) = function_node.child_by_field_name("parameters") else {
            return Vec::new();
        };

        let mut params = Vec::new();
        let mut cursor = parameters.walk();
        for param in parameters.named_children(&mut cursor) {
            if let Some(info) = self.build_param_info(param) {
                params.push(info);
            }
        }
        params
    }

    fn build_param_info(&self, param: Node<'_>) -> Option<FunctionParamInfo> {
        let type_node = param.child_by_field_name("type");
        let name = match param.kind() {
            "identifier" => self.ctx.node_text(param),
            "typed_parameter" | "typed_default_parameter" | "default_parameter" => param
                .child_by_field_name("name")
                .or_else(|| param.child_by_field_name("pattern"))
                .or_else(|| param.child_by_field_name("left"))
                .map_or_else(
                    || self.first_identifier_text(param),
                    |node| self.ctx.node_text(node),
                ),
            "list_splat_pattern" | "dictionary_splat_pattern" => self
                .ctx
                .node_text(param)
                .trim_start_matches('*')
                .to_string(),
            _ => param
                .child_by_field_name("name")
                .or_else(|| param.child_by_field_name("pattern"))
                .or_else(|| param.child_by_field_name("left"))
                .map_or_else(
                    || self.first_identifier_text(param),
                    |node| self.ctx.node_text(node),
                ),
        };

        if name.is_empty() {
            return None;
        }

        Some(FunctionParamInfo {
            name,
            param_type: type_node.map(|node| self.ctx.node_text(node)),
        })
    }

    fn first_identifier_text(&self, node: Node<'_>) -> String {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if matches!(current.kind(), "identifier" | "keyword_identifier") {
                let text = self.ctx.node_text(current);
                if !text.is_empty() {
                    return text;
                }
            }
            let mut cursor = current.walk();
            let children: Vec<_> = current.named_children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
        String::new()
    }
}
