use rayon::prelude::*;
use serde_json::{json, Value};

use crate::analyzer::CodeAnalyzer;
use crate::db::{
    db_path_in_current_dir, file_record_from_path, file_record_without_hash_from_path,
    DbProjectAnalyzer, FileIndexData, IndexSyncPlan,
};
use crate::languages::detect_language;
use crate::project::ProjectAnalyzer;
use crate::utils::progress_bar;

pub(crate) fn build_index(path: &str, language: Option<&str>) -> Value {
    let project = match ProjectAnalyzer::new_with_language(path, language) {
        Ok(project) => project,
        Err(error) => return json!({ "error": error.to_string() }),
    };

    let total_files = project.files.len();
    let progress = progress_bar(total_files, "files", "cyan/blue", "Parsing source files");

    let db_path = match db_path_in_current_dir() {
        Ok(path) => path,
        Err(error) => return json!({ "error": error.to_string() }),
    };

    let existing = match DbProjectAnalyzer::from_db_file(&db_path) {
        Ok(db) if db.is_compatible(path, language).unwrap_or(false) => {
            db.indexed_files().unwrap_or_default()
        }
        Ok(_) => Default::default(),
        Err(_) => Default::default(),
    };

    let indexed: Vec<Result<(crate::db::IndexedFileRecord, Option<FileIndexData>), String>> =
        project
            .files
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

    let update_result =
        DbProjectAnalyzer::update_database(&db_path, path, language, &plan, &db_progress);
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
    existing: Option<&crate::db::IndexedFileRecord>,
) -> anyhow::Result<(crate::db::IndexedFileRecord, Option<FileIndexData>)> {
    let language = detect_language(std::path::Path::new(file))
        .ok_or_else(|| anyhow::anyhow!("Could not detect language for: {}", file))?
        .to_string();
    let quick_record = file_record_without_hash_from_path(file, &language)?;
    if let Some(record) = existing.filter(|record| {
        record.language == quick_record.language
            && record.mtime_nanos == quick_record.mtime_nanos
            && record.size_bytes == quick_record.size_bytes
    }) {
        return Ok((record.clone(), None));
    }
    let file_record = file_record_from_path(file, &language)?;

    let mut analyzer = CodeAnalyzer::new(file)?;
    let snapshot = analyzer.snapshot_for_index();
    Ok((
        file_record.clone(),
        Some(FileIndexData {
            file: file_record,
            snapshot,
        }),
    ))
}
