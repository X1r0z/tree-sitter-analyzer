use std::collections::BTreeSet;
use std::path::Path;

use rayon::prelude::*;

use crate::db::{
    db_path_in_current_dir, file_record_from_metadata, IndexStore, IndexSyncPlan,
    IndexSynchronizer, IndexedFileMetadata, IndexedFileSnapshot,
};
use crate::extractor::CodeExtractor;
use crate::languages::{detect_language, supported_language_names};
use crate::models::IndexInfo;
use crate::utils::{collect_files, progress_bar, resolve_path};

pub(crate) fn ensure_index(path: &str, language: Option<&str>) -> anyhow::Result<()> {
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

pub(crate) fn build_index(path: &str, language: Option<&str>) -> anyhow::Result<IndexInfo> {
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
            store
                .indexed_file_metadata_by_path(&files)
                .unwrap_or_default(),
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

fn build_file_index(
    file: &str,
    existing: Option<&IndexedFileMetadata>,
) -> anyhow::Result<(IndexedFileMetadata, Option<IndexedFileSnapshot>)> {
    use std::fs;

    let language = detect_language(Path::new(file))
        .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file))?
        .name
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
