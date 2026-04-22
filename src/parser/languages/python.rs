use std::cell::Ref;
use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::super::capture::CallCaptureMatch;
use super::super::{
    EnclosingContext, ParseContext, PythonPropertyCallers, PythonPropertyDefinitions,
    PythonPropertyIndexes,
};
use crate::languages::{find_language_info, LanguageEngine, LanguageInfo, ResolvedCall};
use crate::models::{
    AnnotationInfo, FieldInfo, FunctionParamInfo, Location, PythonPropertyCallerInfo,
    PythonPropertyInfo,
};
use crate::parser::call_targets::{
    matches_module_property_target, matches_property_target as call_matches_property_target,
    type_matches_class,
};
use crate::traversal::collect_reachable_bfs;
use crate::utils::select_most_specific_by_line;

type PythonPropertyCallerKey = (String, String, Option<String>, Option<String>, usize, usize);

pub(crate) struct PythonEngine;

pub(crate) static PYTHON_ENGINE: PythonEngine = PythonEngine;

impl LanguageEngine for PythonEngine {
    fn language_info(&self) -> &'static LanguageInfo {
        find_language_info("python").expect("python language info")
    }

    fn normalize_function_node<'a>(&self, _ctx: &ParseContext, node: Node<'a>) -> Node<'a> {
        unwrap_definition_node(node)
    }

    fn normalize_class_node<'a>(&self, _ctx: &ParseContext, node: Node<'a>) -> Node<'a> {
        unwrap_definition_node(node)
    }

    fn function_name(&self, ctx: &ParseContext, node: Node<'_>) -> Option<String> {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = ctx.node_text(name_node);
            if !name.is_empty() {
                return Some(name);
            }
        }
        for i in 0..node.child_count() {
            let child = node.child(i as u32).unwrap();
            if child.kind() == "identifier" {
                let name = ctx.node_text(child);
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
        None
    }

    fn function_params(
        &self,
        ctx: &ParseContext,
        function_node: Node<'_>,
    ) -> Vec<FunctionParamInfo> {
        let Some(parameters) = function_node.child_by_field_name("parameters") else {
            return Vec::new();
        };

        let mut params = Vec::new();
        for i in 0..parameters.named_child_count() {
            let Some(param) = parameters.named_child(i as u32) else {
                continue;
            };
            if let Some(info) = build_parameter_info(ctx, param) {
                params.push(info);
            }
        }
        params
    }

    fn resolve_call<'a>(
        &self,
        ctx: &ParseContext,
        matched: &CallCaptureMatch<'a>,
        enclosing: &EnclosingContext<'a>,
    ) -> ResolvedCall<'a> {
        let _ = enclosing;
        let call_node = matched.call;
        let mut callee = String::new();
        let mut is_method = false;
        let mut object_name: Option<String> = None;
        let mut callee_function_node: Option<Node<'_>> = None;

        if let Some(func_node) = call_node.child_by_field_name("function") {
            callee_function_node = Some(func_node);
            if func_node.kind() == "identifier" {
                callee = ctx.node_text(func_node);
            } else if func_node.kind() == "attribute" {
                is_method = true;
                let (callee_name, resolved_object_name) = split_attribute_parts(ctx, func_node);
                callee = callee_name;
                object_name = resolved_object_name;
            }
        }
        if callee.is_empty() {
            if let Some(callee_cap) = matched.callee {
                callee = ctx.node_text(callee_cap);
            }
            if let Some(method_cap) = matched.method {
                callee = ctx.node_text(method_cap);
                is_method = true;
                if let Some(obj_cap) = matched.object {
                    object_name = Some(ctx.node_text(obj_cap));
                }
            }
        }

        ResolvedCall {
            callee,
            is_method,
            object_name,
            callee_function_node,
        }
    }

    fn is_ref_node(&self, node: Node<'_>) -> bool {
        matches!(
            node.kind(),
            "identifier" | "dotted_name" | "relative_import"
        )
    }

    fn class_fields(
        &self,
        ctx: &ParseContext,
        class_node: Node<'_>,
        class_name: &str,
    ) -> Vec<FieldInfo> {
        struct WalkCtx<'a> {
            parser: &'a ParseContext,
            class_node_id: usize,
            class_name: String,
            fields: Vec<FieldInfo>,
            seen: HashSet<String>,
        }

        fn walk(ctx: &mut WalkCtx, node: Node, inside_method: bool) {
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
                    .map(|child| ctx.parser.node_text(child))
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
                        walk(ctx, child, true);
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

                    let Some(left_node) = child.child_by_field_name("left") else {
                        continue;
                    };
                    let field_type = child
                        .child_by_field_name("type")
                        .map(|node| ctx.parser.node_text(node));
                    let mut name = String::new();

                    if inside_method {
                        if left_node.kind() == "attribute" {
                            let obj_node = left_node.child_by_field_name("object");
                            let attr_node = left_node.child_by_field_name("attribute");
                            if let (Some(obj), Some(attr)) = (obj_node, attr_node) {
                                if ctx.parser.node_text_eq(obj, "self") {
                                    name = ctx.parser.node_text(attr);
                                }
                            }
                        }
                    } else if left_node.kind() == "identifier" {
                        name = ctx.parser.node_text(left_node);
                    }

                    if !name.is_empty() && ctx.seen.insert(name.clone()) {
                        ctx.fields.push(FieldInfo {
                            name,
                            location: ctx.parser.node_location(child),
                            field_type,
                            class_name: Some(ctx.class_name.clone()),
                        });
                    }
                }
                return;
            }

            for i in 0..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    walk(ctx, child, inside_method);
                }
            }
        }

        let mut walk_ctx = WalkCtx {
            parser: ctx,
            class_node_id: class_node.id(),
            class_name: class_name.to_string(),
            fields: Vec::new(),
            seen: HashSet::new(),
        };
        walk(&mut walk_ctx, class_node, false);
        walk_ctx.fields
    }

    fn super_types(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String> {
        let mut super_classes = Vec::new();
        for i in 0..class_node.child_count() {
            let child = class_node.child(i as u32).unwrap();
            if child.kind() != "argument_list" {
                continue;
            }
            for j in 0..child.child_count() {
                let arg = child.child(j as u32).unwrap();
                if matches!(arg.kind(), "identifier" | "attribute") {
                    super_classes.push(ctx.node_text(arg));
                }
            }
        }
        super_classes
    }

    fn annotations(&self, ctx: &ParseContext) -> Vec<AnnotationInfo> {
        let mut annotations = Vec::new();
        let mut stack = vec![ctx.tree().root_node()];

        while let Some(node) = stack.pop() {
            if node.kind() == "decorated_definition" {
                collect_decorators_from_decorated(ctx, node, &mut annotations);
                for i in (0..node.child_count()).rev() {
                    if let Some(child) = node.child(i as u32) {
                        stack.push(child);
                    }
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

    fn include_call(
        &self,
        _ctx: &ParseContext,
        call_node: Node<'_>,
        callee: &str,
        object_name: Option<&str>,
    ) -> bool {
        !should_skip_call(call_node, callee, object_name)
    }

    fn ref_filter(&self, _ctx: &ParseContext, node: Node<'_>) -> bool {
        if node.kind() != "identifier" {
            return true;
        }

        let mut current = node.parent();
        let mut has_import_name_ancestor = false;
        while let Some(parent) = current {
            match parent.kind() {
                "dotted_name" | "relative_import" => has_import_name_ancestor = true,
                "import_statement" | "import_from_statement" => return !has_import_name_ancestor,
                _ => {}
            }
            current = parent.parent();
        }

        true
    }
}

fn ensure_property_indexes_cached(parser: &ParseContext) {
    if parser.language() != "python" {
        return;
    }
    if parser
        .caches
        .borrow()
        .language
        .python
        .property_indexes
        .is_some()
    {
        return;
    }
    let definitions = collect_property_definitions(parser);
    parser.caches.borrow_mut().language.python.property_indexes = Some(PythonPropertyIndexes {
        definitions,
        callers_by_property: None,
    });
}

fn ensure_property_callers_cached(parser: &ParseContext) {
    if parser.language() != "python" {
        return;
    }
    ensure_property_indexes_cached(parser);

    let should_build = parser
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
        let caches = parser.caches.borrow();
        caches
            .language
            .python
            .property_indexes
            .as_ref()
            .expect("python cache")
            .definitions
            .clone()
    };
    let raw_callers = collect_property_callers_raw(parser);
    let callers_by_property = filter_property_callers(parser, &definitions, &raw_callers);
    if let Some(indexes) = parser
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

pub(crate) fn with_cached_property_definitions<R>(
    parser: &ParseContext,
    f: impl FnOnce(&PythonPropertyDefinitions) -> R,
) -> R {
    if parser.language() != "python" {
        return f(&HashSet::new());
    }
    ensure_property_indexes_cached(parser);
    let caches = parser.caches.borrow();
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

fn with_cached_property_callers<R>(
    parser: &ParseContext,
    f: impl FnOnce(&PythonPropertyCallers) -> R,
) -> R {
    if parser.language() != "python" {
        return f(&HashMap::new());
    }
    ensure_property_callers_cached(parser);
    let caches = parser.caches.borrow();
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

pub(crate) fn split_attribute_parts(
    parser: &ParseContext,
    node: Node<'_>,
) -> (String, Option<String>) {
    let mut callee = String::new();
    let mut object_name: Option<String> = None;

    if let Some(attr_node) = node.child_by_field_name("attribute") {
        callee = parser.node_text(attr_node);
    }
    if let Some(obj_node) = node.child_by_field_name("object") {
        object_name = Some(parser.node_text(obj_node));
    }

    (callee, object_name)
}

pub(crate) fn unwrap_definition_node<'a>(node: Node<'a>) -> Node<'a> {
    if node.kind() == "decorated_definition" {
        if let Some(definition) = node.child_by_field_name("definition") {
            return definition;
        }
    }
    node
}

pub(crate) fn should_skip_call(
    call_node: Node<'_>,
    callee: &str,
    object_name: Option<&str>,
) -> bool {
    if callee != "super" || object_name.is_some() {
        return false;
    }

    let Some(parent) = call_node.parent() else {
        return false;
    };
    parent.kind() == "attribute"
        && parent
            .child_by_field_name("object")
            .is_some_and(|object| object.id() == call_node.id())
}

fn build_parameter_info(parser: &ParseContext, param: Node<'_>) -> Option<FunctionParamInfo> {
    let type_node = param.child_by_field_name("type");
    let name = match param.kind() {
        "identifier" => parser.node_text(param),
        "typed_parameter" | "typed_default_parameter" | "default_parameter" => param
            .child_by_field_name("name")
            .or_else(|| param.child_by_field_name("pattern"))
            .or_else(|| param.child_by_field_name("left"))
            .map(|node| parser.node_text(node))
            .unwrap_or_else(|| first_identifier_text(parser, param)),
        "list_splat_pattern" | "dictionary_splat_pattern" => {
            parser.node_text(param).trim_start_matches('*').to_string()
        }
        _ => param
            .child_by_field_name("name")
            .or_else(|| param.child_by_field_name("pattern"))
            .or_else(|| param.child_by_field_name("left"))
            .map(|node| parser.node_text(node))
            .unwrap_or_else(|| first_identifier_text(parser, param)),
    };

    if name.is_empty() {
        return None;
    }

    Some(FunctionParamInfo {
        name,
        param_type: type_node.map(|node| parser.node_text(node)),
    })
}

pub(crate) fn extract_definition_header(
    parser: &ParseContext,
    definition_node: Node<'_>,
) -> String {
    let end_byte = definition_node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or_else(|| definition_node.end_byte());
    parser
        .source_text(definition_node.start_byte(), end_byte)
        .trim_end()
        .to_string()
}

fn collect_decorators_from_decorated(
    parser: &ParseContext,
    decorated_node: Node<'_>,
    annotations: &mut Vec<AnnotationInfo>,
) {
    let definition = decorated_node.child_by_field_name("definition");
    let (target_name, target_type, target_signature) = match definition {
        Some(definition_node) => {
            let name = definition_node
                .child_by_field_name("name")
                .map(|node| parser.node_text(node))
                .unwrap_or_default();
            let target_kind = match definition_node.kind() {
                "function_definition" => {
                    if parser
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
                extract_definition_header(parser, definition_node),
            )
        }
        None => (String::new(), String::new(), String::new()),
    };

    for i in 0..decorated_node.child_count() {
        let child = decorated_node.child(i as u32).unwrap();
        if child.kind() != "decorator" {
            continue;
        }

        let name = extract_decorator_name(parser, child);
        if name.is_empty() {
            continue;
        }

        annotations.push(AnnotationInfo {
            name,
            signature: parser.node_text(child),
            location: parser.node_location(child),
            target_name: target_name.clone(),
            target_type: target_type.clone(),
            target_signature: format!("{}\n{}", parser.node_text(child), target_signature),
        });
    }
}

fn extract_decorator_name(parser: &ParseContext, decorator_node: Node<'_>) -> String {
    for i in 0..decorator_node.child_count() {
        let child = decorator_node.child(i as u32).unwrap();
        match child.kind() {
            "identifier" | "attribute" => return parser.node_text(child),
            "call" => {
                return child
                    .child_by_field_name("function")
                    .map(|node| parser.node_text(node))
                    .unwrap_or_default();
            }
            _ => {}
        }
    }
    String::new()
}

pub(crate) fn collect_property_definitions(parser: &ParseContext) -> PythonPropertyDefinitions {
    if parser.language() != "python" {
        return HashSet::new();
    }

    let mut properties = HashSet::new();
    let mut stack = vec![parser.tree().root_node()];

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
            for i in 0..node.named_child_count() {
                let Some(child) = node.named_child(i as u32) else {
                    continue;
                };
                if child.kind() == "decorator" && parser.node_trimmed_text_eq(child, "@property") {
                    is_property = true;
                    break;
                }
            }

            if is_property {
                properties.insert((
                    parser.node_text(name_node),
                    parser.find_enclosing_context(definition_node).class_name,
                ));
            }
        }

        for i in (0..node.named_child_count()).rev() {
            if let Some(child) = node.named_child(i as u32) {
                stack.push(child);
            }
        }
    }

    properties
}

pub(crate) fn collect_property_callers_raw(parser: &ParseContext) -> PythonPropertyCallers {
    if parser.language() != "python" {
        return HashMap::new();
    }

    let module_binding_types = collect_module_binding_types(parser);
    let mut callers_by_property: HashMap<String, Vec<PythonPropertyCallerInfo>> = HashMap::new();
    let mut seen_callers: HashSet<PythonPropertyCallerKey> = HashSet::new();
    let mut stack = vec![parser.tree().root_node()];

    while let Some(node) = stack.pop() {
        if node.kind() == "attribute" {
            if !is_load_like_property_access(node) {
                continue;
            }
            let (property_name, object_name) = split_attribute_parts(parser, node);
            if property_name.is_empty() {
                continue;
            }
            let enclosing = parser.find_enclosing_context(node);
            let caller = enclosing
                .function_name
                .unwrap_or_else(|| "<module>".to_string());
            let start_line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            let object_type = if caller == "<module>" {
                object_name
                    .as_deref()
                    .and_then(|name| module_binding_types.get(name))
                    .cloned()
            } else {
                None
            };
            let seen_key = (
                property_name.clone(),
                caller.clone(),
                enclosing.class_name.clone(),
                object_name.clone(),
                start_line,
                end_line,
            );
            if seen_callers.insert(seen_key) {
                callers_by_property
                    .entry(property_name.clone())
                    .or_default()
                    .push(PythonPropertyCallerInfo {
                        location: Location {
                            file: parser.file_path().to_string_lossy().into_owned(),
                            start_line,
                            end_line,
                        },
                        property_name: property_name.clone(),
                        caller,
                        caller_class_name: enclosing.class_name.clone(),
                        object_name,
                        object_type,
                    });
            }
        }

        for i in (0..node.named_child_count()).rev() {
            if let Some(child) = node.named_child(i as u32) {
                stack.push(child);
            }
        }
    }

    callers_by_property
}

fn is_load_like_property_access(node: Node<'_>) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "assignment" | "augmented_assignment" => {
                if parent.child_by_field_name("left").is_some_and(|left| {
                    left.start_byte() <= node.start_byte() && node.end_byte() <= left.end_byte()
                }) {
                    return false;
                }
            }
            _ => {}
        }
        current = parent;
    }
    true
}

fn collect_module_binding_types(parser: &ParseContext) -> HashMap<String, String> {
    let mut bindings = HashMap::new();
    let root = parser.tree().root_node();

    for index in 0..root.named_child_count() {
        let Some(node) = root.named_child(index as u32) else {
            continue;
        };
        match node.kind() {
            "expression_statement" => {
                for index in 0..node.named_child_count() {
                    let Some(child) = node.named_child(index as u32) else {
                        continue;
                    };
                    if child.kind() != "assignment" {
                        continue;
                    }
                    let Some(left) = child.child_by_field_name("left") else {
                        continue;
                    };
                    if left.kind() != "identifier" {
                        continue;
                    }
                    let name = parser.node_text(left);
                    if name.is_empty() {
                        continue;
                    }

                    if let Some(type_node) = child.child_by_field_name("type") {
                        let class_name = parser
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

                    let Some(right) = child.child_by_field_name("right") else {
                        bindings.remove(&name);
                        continue;
                    };
                    if let Some(class_name) = infer_module_binding_type(parser, right) {
                        bindings.insert(name, class_name);
                    } else {
                        bindings.remove(&name);
                    }
                }
            }
            "assignment" => {
                let Some(left) = node.child_by_field_name("left") else {
                    continue;
                };
                if left.kind() != "identifier" {
                    continue;
                }
                let name = parser.node_text(left);
                if name.is_empty() {
                    continue;
                }

                if let Some(type_node) = node.child_by_field_name("type") {
                    let class_name = parser
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

                let Some(right) = node.child_by_field_name("right") else {
                    bindings.remove(&name);
                    continue;
                };
                if let Some(class_name) = infer_module_binding_type(parser, right) {
                    bindings.insert(name, class_name);
                } else {
                    bindings.remove(&name);
                }
            }
            _ => {}
        }
    }

    bindings
}

fn infer_module_binding_type(parser: &ParseContext, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "call" => node
            .child_by_field_name("function")
            .and_then(|function| infer_module_binding_type(parser, function)),
        "identifier" | "attribute" => {
            let class_name = parser
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

pub(crate) fn collect_property_infos(parser: &ParseContext) -> Vec<PythonPropertyInfo> {
    with_cached_property_definitions(parser, |properties| {
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

pub(crate) fn collect_property_callers(
    parser: &ParseContext,
    property_name: Option<&str>,
) -> Vec<PythonPropertyCallerInfo> {
    with_cached_property_callers(parser, |callers| {
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

fn filter_property_callers(
    parser: &ParseContext,
    property_definitions: &PythonPropertyDefinitions,
    callers: &PythonPropertyCallers,
) -> PythonPropertyCallers {
    let property_definitions: Vec<_> = property_definitions
        .iter()
        .cloned()
        .map(|(name, class_name)| PythonPropertyInfo { name, class_name })
        .collect();
    let parents_by_class = parser
        .collect_classes()
        .into_iter()
        .map(|class| (class.name, class.super_classes))
        .collect::<HashMap<_, _>>();
    let mut functions = parser.collect_functions(false);
    functions.sort_by_key(|function| (function.location.start_line, function.location.end_line));

    let mut filtered = HashMap::new();
    for (property_name, entries) in callers {
        let matched: Vec<_> = entries
            .iter()
            .filter(|caller| {
                property_caller_matches_known_property(
                    parser,
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

fn property_caller_matches_known_property(
    parser: &ParseContext,
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
                matches_module_property_target(
                    caller.object_name.as_deref(),
                    caller.object_type.as_deref(),
                    class_name,
                )
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
        .map(|class_name| parser.collect_field_infos_for_class(class_name))
        .unwrap_or_default();

    candidates.iter().any(|property| {
        let Some(target_class_name) = property.class_name.as_deref() else {
            return false;
        };

        let receiver_matches =
            if matches!(caller.object_name.as_deref(), Some("self") | Some("cls")) {
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
            };

        receiver_matches
    })
}

fn first_identifier_text(parser: &ParseContext, node: Node<'_>) -> String {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if matches!(current.kind(), "identifier" | "keyword_identifier") {
            let text = parser.node_text(current);
            if !text.is_empty() {
                return text;
            }
        }
        for i in (0..current.named_child_count()).rev() {
            if let Some(child) = current.named_child(i as u32) {
                stack.push(child);
            }
        }
    }
    String::new()
}
