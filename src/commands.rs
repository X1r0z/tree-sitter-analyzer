use std::collections::HashSet;

use serde_json::{json, Value};

use crate::analyzer::CodeAnalyzer;
use crate::hydrator::{hydrate_function_bodies, hydrate_ref_contexts};
use crate::indexer::{build_index, ensure_index};
use crate::models::GraphDirection;
use crate::output::{self, FunctionView};
use crate::utils::{error_response, resolve_path, success_response};

struct CommandContext {
    resolved_path: String,
    analyzer: CodeAnalyzer,
}

impl CommandContext {
    fn load(path: &str, language: Option<&str>) -> anyhow::Result<Self> {
        let resolved_path = resolve_path(path);
        ensure_index(&resolved_path, language)?;
        let analyzer = CodeAnalyzer::from_current_dir(&resolved_path, language)?;
        Ok(Self {
            resolved_path,
            analyzer,
        })
    }

    fn searched_files(&self) -> usize {
        self.analyzer.file_count()
    }
}

pub(crate) fn index(path: &str, language: Option<&str>) -> Value {
    match build_index(path, language) {
        Ok(index) => success_response(&resolve_path(path), index.candidates, output::index(&index)),
        Err(error) => error_response(error),
    }
}

pub(crate) fn functions(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let functions = match context.analyzer.find_functions(query) {
        Ok(functions) => functions,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::functions(&functions, FunctionView::Summary),
    )
}

pub(crate) fn classes(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let classes = match context.analyzer.find_classes(query) {
        Ok(classes) => classes,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::classes(&classes),
    )
}

pub(crate) fn fields(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let fields = match context.analyzer.find_fields(class_name) {
        Ok(fields) => fields,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::fields(&fields),
    )
}

pub(crate) fn imports(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let imports = match context.analyzer.find_imports(query) {
        Ok(imports) => imports,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::imports(&imports),
    )
}

pub(crate) fn annotations(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let annotations = match context.analyzer.find_annotations(query) {
        Ok(annotations) => annotations,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::annotations(&annotations),
    )
}

pub(crate) fn callers(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let callers = match context.analyzer.find_callers(function_name, class_name) {
        Ok(callers) => callers,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::callers(&callers),
    )
}

pub(crate) fn callees(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let callees = match context.analyzer.find_callees(function_name, class_name) {
        Ok(callees) => callees,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::callees(&callees),
    )
}

pub(crate) fn graph(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
    max_depth: usize,
    direction: GraphDirection,
) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let graphs = match context
        .analyzer
        .find_graphs(function_name, class_name, direction, max_depth)
    {
        Ok(graphs) => graphs,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::graphs(&graphs),
    )
}

pub(crate) fn refs(path: &str, language: Option<&str>, name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let refs = match context.analyzer.find_refs(name) {
        Ok(refs) => refs,
        Err(error) => return error_response(error),
    };
    let refs = hydrate_ref_contexts(refs);
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::refs(&refs),
    )
}

pub(crate) fn definition(
    path: &str,
    language: Option<&str>,
    function_name: &str,
    class_name: Option<&str>,
) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let functions = match context.analyzer.find_functions(function_name) {
        Ok(functions) => functions,
        Err(error) => return error_response(error),
    };
    let functions: Vec<_> = functions
        .into_iter()
        .filter(|function| {
            function.name == function_name
                && (class_name.is_none() || function.class_name.as_deref() == class_name)
        })
        .collect();
    if functions.is_empty() {
        return json!({"error": format!("Function '{}' not found", function_name)});
    }
    let functions = hydrate_function_bodies(functions);
    let searched_files = functions
        .iter()
        .map(|function| function.location.file.as_str())
        .collect::<HashSet<_>>()
        .len();
    success_response(
        &context.resolved_path,
        searched_files,
        output::functions(&functions, FunctionView::Definition),
    )
}

pub(crate) fn super_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let super_classes = match context.analyzer.find_super_classes(class_name) {
        Ok(super_classes) => super_classes,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::classes(&super_classes),
    )
}

pub(crate) fn sub_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let sub_classes = match context.analyzer.find_sub_classes(class_name) {
        Ok(sub_classes) => sub_classes,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.resolved_path,
        context.searched_files(),
        output::classes(&sub_classes),
    )
}
