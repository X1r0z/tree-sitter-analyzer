use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::languages::QueryKind;
use crate::models::{ClassInfo, FieldInfo, FunctionInfo};
use crate::parser::capture;

use super::enclosing::{ClassRange, EnclosingIntervalIndex, FunctionRange};
use super::symbols::{class_kind, class_methods};
use super::ParseContext;

#[derive(Clone, Default)]
pub(crate) struct ClassSnapshotData {
    pub(crate) classes: Vec<ClassInfo>,
    pub(crate) fields: Vec<FieldInfo>,
    pub(crate) field_infos_by_class: HashMap<String, Vec<FieldInfo>>,
}

#[derive(Clone)]
pub(super) struct StructuralIndexData {
    pub(super) functions_without_bodies: Vec<FunctionInfo>,
    pub(super) class_snapshot: ClassSnapshotData,
    pub(super) enclosing_interval_index: EnclosingIntervalIndex,
}

struct FunctionSnapshotEntry {
    start_byte: usize,
    end_byte: usize,
    function: FunctionInfo,
}

struct ClassSnapshotEntry {
    start_byte: usize,
    end_byte: usize,
    class: ClassInfo,
}

impl ParseContext {
    pub(crate) fn collect_functions(&self, include_body: bool) -> Vec<FunctionInfo> {
        if !include_body {
            return self
                .with_structural_index(|index| index.functions_without_bodies.clone())
                .unwrap_or_default();
        }
        self.build_function_entries(true)
            .into_iter()
            .map(|entry| entry.function)
            .collect()
    }

    pub(crate) fn collect_classes(&self) -> Vec<ClassInfo> {
        self.with_structural_index(|index| index.class_snapshot.classes.clone())
            .unwrap_or_default()
    }

    pub(crate) fn collect_fields_for_class(&self, class_name: &str) -> Vec<FieldInfo> {
        self.with_structural_index(|index| {
            index
                .class_snapshot
                .field_infos_by_class
                .get(class_name)
                .cloned()
        })
        .flatten()
        .unwrap_or_default()
    }

    pub(crate) fn collect_class_snapshot(&self) -> ClassSnapshotData {
        self.with_structural_index(|index| index.class_snapshot.clone())
            .unwrap_or_default()
    }

    pub(super) fn ensure_structural_index(&self) {
        if self.caches.borrow().common.structural_index.is_some() {
            return;
        }
        let function_entries = self.build_function_entries(false);
        let functions_without_bodies = function_entries
            .iter()
            .map(|entry| entry.function.clone())
            .collect::<Vec<_>>();

        let methods_by_class = self.receiver_methods_by_class(&functions_without_bodies);
        let (class_entries, field_infos_by_class) = self.build_class_entries(&methods_by_class);

        let enclosing_interval_index = build_interval_index(function_entries, &class_entries);
        let class_snapshot = build_class_snapshot(&class_entries, field_infos_by_class);

        let structural_index = StructuralIndexData {
            functions_without_bodies,
            class_snapshot,
            enclosing_interval_index,
        };
        self.caches.borrow_mut().common.structural_index = Some(structural_index);
    }

    /// Builds the structural index if needed, then runs `f` against it.
    /// Returns `None` only when the index could not be built.
    fn with_structural_index<T>(&self, f: impl FnOnce(&StructuralIndexData) -> T) -> Option<T> {
        self.ensure_structural_index();
        self.caches
            .borrow()
            .common
            .structural_index
            .as_ref()
            .map(f)
    }

    fn build_function_entries(&self, include_body: bool) -> Vec<FunctionSnapshotEntry> {
        let mut func_pairs: Vec<(Node<'_>, Node<'_>)> =
            capture::collect_capture_pairs(self, QueryKind::Function, "function", "name");
        func_pairs.sort_by_key(|(f, _)| (f.start_byte(), std::cmp::Reverse(f.end_byte())));

        let mut entries = Vec::new();
        let mut seen = HashSet::new();

        for (func_node, name_node) in func_pairs {
            let function_node = self.engine().normalize_function_node(self, func_node);
            let name = self
                .cached_function_name(function_node)
                .unwrap_or_else(|| self.node_text(name_node));
            if name.is_empty() {
                continue;
            }
            let class_name = self.compute_enclosing_names(function_node).class_name;
            let key = (
                function_node.start_byte(),
                function_node.end_byte(),
                name.clone(),
                class_name.clone(),
            );
            if !seen.insert(key) {
                continue;
            }
            entries.push(FunctionSnapshotEntry {
                start_byte: function_node.start_byte(),
                end_byte: function_node.end_byte(),
                function: FunctionInfo {
                    name,
                    location: self.node_location(function_node),
                    body: if include_body {
                        self.node_text(function_node)
                    } else {
                        String::new()
                    },
                    class_name,
                    params: self.collect_function_params(function_node),
                },
            });
        }

        entries
    }

    fn build_class_entries(
        &self,
        methods_by_class: &HashMap<String, Vec<String>>,
    ) -> (Vec<ClassSnapshotEntry>, HashMap<String, Vec<FieldInfo>>) {
        let mut class_pairs: Vec<(Node<'_>, Node<'_>)> =
            capture::collect_capture_pairs(self, QueryKind::Class, "class", "name");
        class_pairs.sort_by_key(|(class_node, _)| {
            (
                class_node.start_byte(),
                std::cmp::Reverse(class_node.end_byte()),
            )
        });

        let mut class_entries = Vec::new();
        let mut active_ranges: Vec<usize> = Vec::new();
        let mut seen = HashSet::new();
        let allow_nested_classes = matches!(self.language(), "python" | "java");
        let mut chosen_fields_by_class: HashMap<String, (usize, usize, Vec<FieldInfo>)> =
            HashMap::new();

        for (class_node, name_node) in class_pairs {
            let class_node = self.engine().normalize_class_node(self, class_node);
            let name = self
                .cached_class_name(class_node)
                .unwrap_or_else(|| self.node_text(name_node));
            if name.is_empty() {
                continue;
            }
            let start = class_node.start_byte();
            let end = class_node.end_byte();
            if !seen.insert((start, end, name.clone())) {
                continue;
            }
            while let Some(&active_end) = active_ranges.last() {
                if start >= active_end {
                    active_ranges.pop();
                } else {
                    break;
                }
            }
            let is_nested = !active_ranges.is_empty();
            if is_nested && !allow_nested_classes {
                continue;
            }
            active_ranges.push(end);

            let field_infos = self.engine().class_fields(self, class_node, &name);
            let field_names = field_infos.iter().map(|field| field.name.clone()).collect();
            record_class_fields(
                &mut chosen_fields_by_class,
                &name,
                end - start,
                start,
                field_infos,
            );

            let mut method_names = class_methods(self, class_node);
            if self.language() == "go" {
                if let Some(go_methods) = methods_by_class.get(&name) {
                    method_names.extend(go_methods.iter().cloned());
                    method_names.sort_unstable();
                    method_names.dedup();
                }
            }
            let super_class_names = self.engine().super_classes(self, class_node);

            class_entries.push(ClassSnapshotEntry {
                start_byte: start,
                end_byte: end,
                class: ClassInfo {
                    name,
                    kind: class_kind(self, class_node),
                    location: self.node_location(class_node),
                    methods: method_names,
                    fields: field_names,
                    super_classes: super_class_names,
                },
            });
        }

        let field_infos_by_class = chosen_fields_by_class
            .into_iter()
            .map(|(name, (_, _, fields))| (name, fields))
            .collect::<HashMap<_, _>>();

        (class_entries, field_infos_by_class)
    }

    /// Maps each Go type to its receiver-method names (sorted, deduped).
    /// Non-Go languages get methods directly from the class body, so this is empty.
    fn receiver_methods_by_class(&self, functions: &[FunctionInfo]) -> HashMap<String, Vec<String>> {
        let mut methods_by_class: HashMap<String, Vec<String>> = HashMap::new();
        if self.language() != "go" {
            return methods_by_class;
        }
        for function in functions {
            if let Some(class_name) = function.class_name.clone() {
                methods_by_class
                    .entry(class_name)
                    .or_default()
                    .push(function.name.clone());
            }
        }
        for methods in methods_by_class.values_mut() {
            methods.sort_unstable();
            methods.dedup();
        }
        methods_by_class
    }
}

/// Records `field_infos` as the chosen fields for `name`, keeping whichever
/// declaration is smaller (and earlier on ties) so the tightest enclosing
/// class wins when a name is declared more than once.
fn record_class_fields(
    chosen: &mut HashMap<String, (usize, usize, Vec<FieldInfo>)>,
    name: &str,
    class_size: usize,
    start: usize,
    field_infos: Vec<FieldInfo>,
) {
    match chosen.get(name) {
        Some((best_size, best_start, _)) if (*best_size, *best_start) <= (class_size, start) => {}
        _ => {
            chosen.insert(name.to_string(), (class_size, start, field_infos));
        }
    }
}

fn build_class_snapshot(
    class_entries: &[ClassSnapshotEntry],
    field_infos_by_class: HashMap<String, Vec<FieldInfo>>,
) -> ClassSnapshotData {
    let classes = class_entries
        .iter()
        .map(|entry| entry.class.clone())
        .collect::<Vec<_>>();
    let fields = classes
        .iter()
        .flat_map(|class| {
            field_infos_by_class
                .get(&class.name)
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    ClassSnapshotData {
        classes,
        fields,
        field_infos_by_class,
    }
}

fn build_interval_index(
    function_entries: Vec<FunctionSnapshotEntry>,
    class_entries: &[ClassSnapshotEntry],
) -> EnclosingIntervalIndex {
    let function_ranges = function_entries
        .into_iter()
        .map(|entry| FunctionRange {
            start_byte: entry.start_byte,
            end_byte: entry.end_byte,
            function_name: entry.function.name,
        })
        .collect();
    let class_ranges = class_entries
        .iter()
        .map(|entry| ClassRange {
            start_byte: entry.start_byte,
            end_byte: entry.end_byte,
            class_name: entry.class.name.clone(),
        })
        .collect();
    EnclosingIntervalIndex {
        function_ranges,
        class_ranges,
    }
}
