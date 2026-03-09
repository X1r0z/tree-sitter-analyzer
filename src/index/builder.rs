use rayon::prelude::*;
use serde_json::{json, Value};

use crate::backend::SourceAnalyzer;
use crate::extractor::CodeExtractor;
use crate::index::{
    db_path_in_current_dir, file_record_from_path, file_record_from_path_without_hash,
    FileIndexData, IndexStore, IndexSyncPlan, IndexSynchronizer,
};
use crate::languages::detect_language;
use crate::utils::progress_bar;

pub(crate) fn build_index(path: &str, language: Option<&str>) -> Value {
    let source_backend = match SourceAnalyzer::new_with_language(path, language) {
        Ok(source_backend) => source_backend,
        Err(error) => return json!({ "error": error.to_string() }),
    };

    let total_files = source_backend.file_count();
    let progress = progress_bar(total_files, "files", "cyan/blue", "Parsing source files");

    let db_path = match db_path_in_current_dir() {
        Ok(path) => path,
        Err(error) => return json!({ "error": error.to_string() }),
    };

    let existing = match IndexStore::open(&db_path) {
        Ok(store) if store.is_compatible_with(path, language).unwrap_or(false) => {
            store.indexed_files_by_path().unwrap_or_default()
        }
        Ok(_) => Default::default(),
        Err(_) => Default::default(),
    };

    let indexed: Vec<Result<(crate::index::IndexedFileRecord, Option<FileIndexData>), String>> =
        source_backend
            .files()
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

    let db_progress = progress_bar(0, "steps", "green/blue", "Persisting index data");

    let update_result = IndexSynchronizer::sync(&db_path, path, language, &plan, &db_progress);
    db_progress.finish_and_clear();

    if let Err(error) = update_result {
        return json!({ "error": error.to_string() });
    }

    json!({
        "path": path,
        "database": db_path.to_string_lossy(),
        "discovered_files": total_files,
        "indexed_files": plan.current_files.len(),
        "reparsed_files": plan.changed_snapshots.len(),
        "failed_files": errors.len(),
        "errors": errors,
    })
}

fn build_file_index(
    file: &str,
    existing: Option<&crate::index::IndexedFileRecord>,
) -> anyhow::Result<(crate::index::IndexedFileRecord, Option<FileIndexData>)> {
    let language = detect_language(std::path::Path::new(file))
        .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file))?
        .to_string();
    let record_without_hash = file_record_from_path_without_hash(file, &language)?;
    if let Some(record) = existing.filter(|record| {
        record.language == record_without_hash.language
            && record.mtime_nanos == record_without_hash.mtime_nanos
            && record.size_bytes == record_without_hash.size_bytes
    }) {
        return Ok((record.clone(), None));
    }
    let file_record = file_record_from_path(file, &language)?;

    let mut extractor = CodeExtractor::new(file)?;
    let snapshot = extractor.snapshot_for_index();
    Ok((
        file_record.clone(),
        Some(FileIndexData {
            file: file_record,
            snapshot,
        }),
    ))
}
