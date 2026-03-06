use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::analyzer::CodeAnalyzer;
use crate::db::{db_path_in_current_dir, file_record_from_path, DbProjectAnalyzer, FileIndexData};
use crate::project::ProjectAnalyzer;

pub(crate) fn build_index(path: &str, language: Option<&str>) -> Value {
    let project = match ProjectAnalyzer::new_with_language(path, language) {
        Ok(project) => project,
        Err(error) => return json!({ "error": error.to_string() }),
    };

    let total_files = project.files.len();
    let progress = ProgressBar::new(total_files as u64);
    progress.set_style(
        ProgressStyle::with_template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("##-"),
    );

    let indexed: Vec<Result<FileIndexData, String>> = project
        .files
        .par_iter()
        .map(|file| {
            let result = build_file_index(file);
            progress.inc(1);
            result.map_err(|error| format!("{}: {}", file, error))
        })
        .collect();
    progress.finish_and_clear();

    let mut snapshots = Vec::new();
    let mut errors = Vec::new();
    for entry in indexed {
        match entry {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(error) => errors.push(error),
        }
    }

    let db_path = match db_path_in_current_dir() {
        Ok(path) => path,
        Err(error) => return json!({ "error": error.to_string() }),
    };

    if let Err(error) = DbProjectAnalyzer::rebuild_database(&db_path, path, language, &snapshots) {
        return json!({ "error": error.to_string() });
    }

    json!({
        "path": path,
        "database": db_path.to_string_lossy(),
        "files_discovered": total_files,
        "indexed_files": snapshots.len(),
        "failed_files": errors.len(),
        "errors": errors,
    })
}

fn build_file_index(file: &str) -> anyhow::Result<FileIndexData> {
    let mut analyzer = CodeAnalyzer::new(file)?;
    let language = analyzer.parser.language.clone();
    let snapshot = analyzer.snapshot_for_index();
    let file = file_record_from_path(file, &language)?;
    Ok(FileIndexData { file, snapshot })
}
