use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::Path;

use rayon::prelude::*;
use serde_json::{json, Value};

use crate::analyzers::StoreAnalyzer;
use crate::db::{
    db_path_in_current_dir, file_record_metadata, file_record_with_hash_from_source, FileIndexData,
    IndexStore, IndexSyncPlan, IndexSynchronizer, IndexedFileRecord,
};
use crate::extractor::CodeExtractor;
use crate::languages::detect_language;
use crate::models::{
    FunctionInfo, FunctionKey, GraphDirection, IndexInfo, Location, RefInfo, RefKey,
};
use crate::output::{self, FunctionView};
use crate::utils::{find_files, progress_bar};

struct CommandContext {
    real_path: String,
    store: StoreAnalyzer,
}

impl CommandContext {
    fn load(path: &str, language: Option<&str>) -> anyhow::Result<Self> {
        let real_path = resolve_path(path);
        ensure_index_for_query(&real_path, language)?;
        let store = StoreAnalyzer::from_current_dir(&real_path, language)?;
        Ok(Self { real_path, store })
    }

    fn searched_files(&self) -> usize {
        self.store.file_count()
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
    let functions = match context.store.find_functions(query) {
        Ok(functions) => functions,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        output::functions(&functions, FunctionView::Summary),
    )
}

pub(crate) fn classes(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let classes = match context.store.find_classes(query) {
        Ok(classes) => classes,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        output::classes(&classes),
    )
}

pub(crate) fn fields(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let fields = match context.store.find_fields(class_name) {
        Ok(fields) => fields,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        output::fields(&fields),
    )
}

pub(crate) fn imports(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let imports = match context.store.find_imports(query) {
        Ok(imports) => imports,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        output::imports(&imports),
    )
}

pub(crate) fn annotations(path: &str, language: Option<&str>, query: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let annotations = match context.store.find_annotations(query) {
        Ok(annotations) => annotations,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
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
    let callers = match context.store.find_callers(function_name, class_name) {
        Ok(callers) => callers,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
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
    let callees = match context.store.find_callees(function_name, class_name) {
        Ok(callees) => callees,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
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
        .store
        .find_graphs(function_name, class_name, direction, max_depth)
    {
        Ok(graphs) => graphs,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        output::graphs(&graphs),
    )
}

pub(crate) fn refs(path: &str, language: Option<&str>, name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let refs = match context.store.find_refs(name) {
        Ok(refs) => refs,
        Err(error) => return error_response(error),
    };
    let refs = hydrate_ref_contexts(refs);
    success_response(
        &context.real_path,
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
    let functions = match context.store.find_functions(function_name) {
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
        &context.real_path,
        searched_files,
        output::functions(&functions, FunctionView::Definition),
    )
}

pub(crate) fn super_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let super_classes = match context.store.find_super_classes(class_name) {
        Ok(super_classes) => super_classes,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        output::classes(&super_classes),
    )
}

pub(crate) fn sub_classes(path: &str, language: Option<&str>, class_name: &str) -> Value {
    let context = match CommandContext::load(path, language) {
        Ok(context) => context,
        Err(error) => return error_response(error),
    };
    let sub_classes = match context.store.find_sub_classes(class_name) {
        Ok(sub_classes) => sub_classes,
        Err(error) => return error_response(error),
    };
    success_response(
        &context.real_path,
        context.searched_files(),
        output::classes(&sub_classes),
    )
}

fn ensure_index_for_query(path: &str, language: Option<&str>) -> anyhow::Result<()> {
    let db_path = db_path_in_current_dir()?;
    let needs_index = if !db_path.exists() {
        true
    } else {
        match IndexStore::open(&db_path) {
            Ok(store) => !store.is_compatible_with(path, language)?,
            Err(_) => true,
        }
    };

    if needs_index {
        build_index(path, language)?;
    }
    Ok(())
}

fn build_file_index(
    file: &str,
    existing: Option<&IndexedFileRecord>,
) -> anyhow::Result<(IndexedFileRecord, Option<FileIndexData>)> {
    use std::fs;

    let language = detect_language(Path::new(file))
        .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file))?
        .to_string();
    let record_without_hash = file_record_metadata(file, &language)?;
    if let Some(record) = existing.filter(|record| {
        record.language == record_without_hash.language
            && record.mtime_nanos == record_without_hash.mtime_nanos
            && record.size_bytes == record_without_hash.size_bytes
    }) {
        return Ok((record.clone(), None));
    }
    let source = fs::read(file)?;
    let file_record = file_record_with_hash_from_source(file, &language, &source)?;

    let mut extractor = CodeExtractor::from_source(file, source)?;
    let snapshot = extractor.build_snapshot();
    Ok((
        file_record.clone(),
        Some(FileIndexData {
            file: file_record,
            snapshot,
        }),
    ))
}

fn build_index(path: &str, language: Option<&str>) -> anyhow::Result<IndexInfo> {
    let real_path = resolve_path(path);
    let root = Path::new(&real_path);
    anyhow::ensure!(root.exists(), "Path not found: {}", path);
    anyhow::ensure!(root.is_dir(), "Path must be a directory: {}", path);

    let files = find_files(&real_path, language);
    let total_files = files.len();
    let progress = progress_bar(total_files, "files", "cyan/blue", "Parsing source files");

    let db_path = db_path_in_current_dir()?;
    let existing = match IndexStore::open(&db_path) {
        Ok(store)
            if store
                .is_compatible_with(&real_path, language)
                .unwrap_or(false) =>
        {
            store.indexed_files_by_paths(&files).unwrap_or_default()
        }
        Ok(_) | Err(_) => Default::default(),
    };

    let indexed: Vec<Result<(IndexedFileRecord, Option<FileIndexData>), String>> = files
        .par_iter()
        .map(|file| {
            let result = build_file_index(file, existing.get(file));
            progress.inc(1);
            result.map_err(|error| format!("{}: {}", file, error))
        })
        .collect();
    progress.finish_and_clear();

    let mut current_files = Vec::new();
    let mut snapshots = Vec::new();
    let mut errors = Vec::new();
    for entry in indexed {
        match entry {
            Ok((record, snapshot)) => {
                current_files.push(record);
                if let Some(snapshot) = snapshot {
                    snapshots.push(snapshot);
                }
            }
            Err(error) => errors.push(error),
        }
    }

    let plan = IndexSyncPlan {
        current_files,
        changed_files: snapshots,
    };

    let db_progress = progress_bar(0, "steps", "green/blue", "Persisting index data");
    IndexSynchronizer::sync(&db_path, &real_path, language, &plan, &db_progress)?;
    db_progress.finish_and_clear();

    Ok(IndexInfo {
        database: db_path.to_string_lossy().to_string(),
        candidates: total_files,
        indexed: plan.current_files.len(),
        reparsed: plan.changed_files.len(),
        failed: errors.len(),
        errors,
    })
}

fn hydrate_function_bodies(candidates: Vec<FunctionInfo>) -> Vec<FunctionInfo> {
    hydrate_candidates(
        candidates,
        |candidate: &FunctionInfo| FunctionKey::from(candidate),
        |candidate| candidate.location.file.as_str(),
        |extractor, expected| {
            expected
                .iter()
                .flat_map(|key| {
                    extractor
                        .find_function_definitions(&key.name, key.class_name.as_deref())
                        .into_iter()
                        .filter(|function| FunctionKey::from(function) == *key)
                })
                .collect()
        },
    )
}

fn hydrate_ref_contexts(candidates: Vec<RefInfo>) -> Vec<RefInfo> {
    hydrate_candidates(
        candidates,
        |candidate: &RefInfo| RefKey::from(candidate),
        |candidate| candidate.location.file.as_str(),
        |extractor, expected| {
            let expected_refs: Vec<_> = expected
                .iter()
                .map(|key| RefInfo {
                    name: key.name.clone(),
                    node_type: key.node_type.clone(),
                    location: Location {
                        file: key.file.clone(),
                        start_line: key.start_line,
                        end_line: key.end_line,
                    },
                    start_column: key.start_column,
                    end_column: key.end_column,
                    context: String::new(),
                })
                .collect();
            extractor.hydrate_refs(&expected_refs)
        },
    )
}

fn hydrate_candidates<K, Candidate, KeyOf, FileOf, Resolve>(
    candidates: Vec<Candidate>,
    key_of: KeyOf,
    file_of: FileOf,
    resolve: Resolve,
) -> Vec<Candidate>
where
    K: Clone + Eq + Hash + Send + Sync,
    Candidate: Clone + Send + Sync,
    KeyOf: for<'a> Fn(&'a Candidate) -> K + Sync,
    FileOf: for<'a> Fn(&'a Candidate) -> &'a str,
    Resolve: Fn(&mut CodeExtractor, &HashSet<K>) -> Vec<Candidate> + Sync + Send,
{
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut expected_keys_by_file: HashMap<String, HashSet<K>> = HashMap::new();
    for candidate in &candidates {
        expected_keys_by_file
            .entry(file_of(candidate).to_string())
            .or_default()
            .insert(key_of(candidate));
    }

    let expected_keys_by_file: Vec<_> = expected_keys_by_file.into_iter().collect();
    let resolved: HashMap<K, Candidate> = expected_keys_by_file
        .par_iter()
        .flat_map(|(file, expected)| {
            let mut extractor = match CodeExtractor::new(file) {
                Ok(extractor) => extractor,
                Err(_) => return Vec::new(),
            };
            resolve(&mut extractor, expected)
                .into_iter()
                .map(|candidate| (key_of(&candidate), candidate))
                .collect::<Vec<_>>()
        })
        .collect();

    candidates
        .into_iter()
        .map(|candidate| {
            resolved
                .get(&key_of(&candidate))
                .cloned()
                .unwrap_or(candidate)
        })
        .collect()
}

fn resolve_path(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(path) => path.to_string_lossy().to_string(),
        Err(_) => {
            let path_ref = Path::new(path);
            if path_ref.is_absolute() {
                path.to_string()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path).to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.to_string())
            }
        }
    }
}

fn success_response(path: &str, searched_files: usize, results: Value) -> Value {
    let count = match &results {
        Value::Array(items) => items.len(),
        _ => 0,
    };
    json!({
        "meta": {
            "root": path,
            "files": searched_files,
            "count": count,
        },
        "results": results,
    })
}

fn error_response(error: impl std::fmt::Display) -> Value {
    json!({ "error": error.to_string() })
}
