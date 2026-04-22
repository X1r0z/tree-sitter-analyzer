use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::Hash;
use std::path::Path;

use rayon::prelude::*;
use serde_json::{json, Value};

use crate::analyzer::CodeAnalyzer;
use crate::db::{
    db_path_in_current_dir, file_record_from_metadata, IndexStore, IndexSyncPlan,
    IndexSynchronizer, IndexedFileMetadata, IndexedFileSnapshot,
};
use crate::extractor::CodeExtractor;
use crate::languages::{detect_language, supported_language_names};
use crate::models::{
    FunctionInfo, FunctionKey, GraphDirection, IndexInfo, Location, RefInfo, RefKey,
};
use crate::output::{self, FunctionView};
use crate::utils::{collect_files, progress_bar};

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

fn ensure_index(path: &str, language: Option<&str>) -> anyhow::Result<()> {
    let db_path = db_path_in_current_dir()?;
    if !db_path.exists() {
        build_index(path, language)?;
        return Ok(());
    }

    let missing_languages = match IndexStore::open(&db_path) {
        Ok(store) => store.missing_languages(path, language)?,
        Err(_) => {
            build_index(path, language)?;
            return Ok(());
        }
    };

    if missing_languages.is_empty() {
        return Ok(());
    }

    if language.is_none() && missing_languages.len() > 1 {
        for missing_language in missing_languages {
            build_index(path, Some(missing_language.as_str()))?;
        }
    } else if let Some(missing_language) = missing_languages.first() {
        build_index(path, Some(missing_language.as_str()))?;
    }
    Ok(())
}

fn build_file_index(
    file: &str,
    existing: Option<&IndexedFileMetadata>,
) -> anyhow::Result<(IndexedFileMetadata, Option<IndexedFileSnapshot>)> {
    use std::fs;

    let language = detect_language(Path::new(file))
        .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file))?
        .to_string();
    let metadata = fs::metadata(file)?;
    let record_without_hash = file_record_from_metadata(file, &language, &metadata)?;
    if let Some(record) = existing.filter(|record| {
        record.language == record_without_hash.language
            && record.mtime_nanos == record_without_hash.mtime_nanos
            && record.size_bytes == record_without_hash.size_bytes
    }) {
        return Ok((record.clone(), None));
    }
    let source = fs::read(file)?;
    let mut file_record = record_without_hash;
    file_record.content_hash = blake3::hash(&source).to_hex().to_string();

    let mut extractor = CodeExtractor::from_source(file, source)?;
    let snapshot = extractor.build_snapshot();
    Ok((
        file_record.clone(),
        Some(IndexedFileSnapshot {
            metadata: file_record,
            snapshot,
        }),
    ))
}

fn build_index(path: &str, language: Option<&str>) -> anyhow::Result<IndexInfo> {
    let resolved_path = resolve_path(path);
    let root = Path::new(&resolved_path);
    anyhow::ensure!(root.exists(), "Path not found: {}", path);
    anyhow::ensure!(root.is_dir(), "Path must be a directory: {}", path);

    let discovery = collect_files(&resolved_path, language);
    let files = discovery.files;
    let total_files = files.len();
    let db_path = db_path_in_current_dir()?;
    let language_scope = language_scope_label(language, &discovery.languages);
    let (incremental, existing) = match IndexStore::open(&db_path) {
        Ok(store) if store.matches_root_path(&resolved_path).unwrap_or(false) => (
            true,
            store.indexed_file_metadata_by_path(&files).unwrap_or_default(),
        ),
        Ok(_) | Err(_) => (false, Default::default()),
    };
    let parse_message = format!(
        "{} index [{}] | Parsing files",
        index_mode_label(incremental),
        language_scope
    );
    let progress = progress_bar(total_files, "files", "cyan/blue", &parse_message);

    let indexed: Vec<Result<(IndexedFileMetadata, Option<IndexedFileSnapshot>), String>> = files
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
        changed_snapshots: snapshots,
    };

    let persist_message = format!(
        "{} index [{}] | Writing index",
        index_mode_label(incremental),
        language_scope
    );
    let db_progress = progress_bar(0, "steps", "green/blue", &persist_message);
    IndexSynchronizer::sync(&db_path, &resolved_path, language, &plan, &db_progress)?;
    db_progress.finish_and_clear();

    Ok(IndexInfo {
        database: db_path.to_string_lossy().to_string(),
        candidates: total_files,
        indexed: plan.current_files.len(),
        reparsed: plan.changed_snapshots.len(),
        failed: errors.len(),
        errors,
    })
}

fn index_mode_label(incremental: bool) -> &'static str {
    if incremental {
        "Incremental"
    } else {
        "Full"
    }
}

fn language_scope_label(language: Option<&str>, languages: &BTreeSet<String>) -> String {
    match language {
        Some(language) => language.to_string(),
        None => {
            if languages.is_empty() {
                supported_language_names().join(", ")
            } else {
                languages.iter().cloned().collect::<Vec<_>>().join(", ")
            }
        }
    }
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
                        .collect_function_definitions(&key.name, key.class_name.as_deref())
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
