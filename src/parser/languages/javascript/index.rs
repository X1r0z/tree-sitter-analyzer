use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use tree_sitter::Node;

use crate::models::FieldInfo;
use crate::parser::ParseContext;

use super::semantic_facts::semantic_facts;
use super::type_helper::JsTypeHelper;

#[derive(Debug, Clone, Default)]
pub(crate) struct JsTypeIndex {
    pub(crate) class_names: HashSet<String>,
    pub(crate) field_types_by_class: HashMap<String, HashMap<String, Option<String>>>,
    pub(crate) field_infos_by_class: HashMap<String, Vec<FieldInfo>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct JsReceiverIndex {
    pub(crate) class_names: HashSet<String>,
    pub(crate) field_targets_by_class: HashMap<String, HashMap<String, Vec<String>>>,
}

impl JsReceiverIndex {
    pub(super) fn symbolic_targets(
        &self,
        ctx: &ParseContext,
        call_node: Node<'_>,
        target: &str,
    ) -> Vec<String> {
        if target.is_empty() {
            return Vec::new();
        }
        if self.class_names.contains(target) {
            return vec![target.to_string()];
        }
        if target == "this" {
            return ctx
                .find_enclosing_context(call_node)
                .class_name
                .into_iter()
                .collect();
        }
        if let Some(chain) = target.strip_prefix("this.") {
            return ctx
                .find_enclosing_context(call_node)
                .class_name
                .into_iter()
                .flat_map(|class_name| self.field_chain(&class_name, chain))
                .collect();
        }
        Vec::new()
    }

    fn field_chain(&self, root_class: &str, chain: &str) -> Vec<String> {
        let mut current = vec![root_class.to_string()];
        for segment in chain.split('.') {
            if segment.is_empty() {
                return Vec::new();
            }
            let mut next = Vec::new();
            for class_name in &current {
                next.extend(self.field_targets(class_name, segment));
            }
            next.sort_unstable();
            next.dedup();
            if next.is_empty() {
                return Vec::new();
            }
            current = next;
        }
        current
    }

    fn field_targets(&self, class_name: &str, field_name: &str) -> Vec<String> {
        self.field_targets_by_class
            .get(class_name)
            .and_then(|fields| fields.get(field_name))
            .cloned()
            .unwrap_or_default()
    }
}

pub(crate) fn type_index(ctx: &ParseContext) -> Rc<JsTypeIndex> {
    if let Some(cached) = ctx.caches.borrow().language.js.type_index.as_ref() {
        return Rc::clone(cached);
    }

    let semantic = semantic_facts(ctx);
    let index = Rc::new(JsTypeIndex {
        class_names: semantic.class_names.clone(),
        field_types_by_class: semantic.field_types_by_class.clone(),
        field_infos_by_class: semantic.field_infos_by_class.clone(),
    });
    ctx.caches.borrow_mut().language.js.type_index = Some(Rc::clone(&index));
    index
}

pub(crate) fn receiver_index(ctx: &ParseContext) -> Rc<JsReceiverIndex> {
    if let Some(cached) = { ctx.caches.borrow().language.js.receiver_index.clone() } {
        return cached;
    }

    let semantic = semantic_facts(ctx);
    let mut field_targets_by_class = HashMap::new();
    for (class_name, fields) in &semantic.field_types_by_class {
        let mut targets_by_field = HashMap::new();
        for (field_name, field_type) in fields {
            let targets = field_type
                .as_deref()
                .map(|field_type| {
                    JsTypeHelper::extract_class_targets(&semantic.class_names, field_type)
                })
                .unwrap_or_default();
            if !targets.is_empty() {
                targets_by_field.insert(field_name.clone(), targets);
            }
        }
        if !targets_by_field.is_empty() {
            field_targets_by_class.insert(class_name.clone(), targets_by_field);
        }
    }

    let index = Rc::new(JsReceiverIndex {
        class_names: semantic.class_names.clone(),
        field_targets_by_class,
    });
    ctx.caches.borrow_mut().language.js.receiver_index = Some(Rc::clone(&index));
    index
}
