use std::collections::{BTreeSet, HashSet};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Mutex;

use ignore::WalkState;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use serde_json::{json, Value};

use crate::languages::detect_language;

#[derive(Debug, Default)]
pub struct FileDiscovery {
    pub files: Vec<String>,
    pub languages: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, Eq, Hash, PartialEq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    path: String,
}

#[derive(Debug, Clone)]
struct DiscoveredFile {
    path: String,
    identity: FileIdentity,
}

#[derive(Default)]
struct LocalFileCollection {
    files: Vec<DiscoveredFile>,
    languages: BTreeSet<String>,
}

struct ThreadLocalFileCollector<'a> {
    shared: &'a Mutex<LocalFileCollection>,
    local: LocalFileCollection,
}

impl<'a> ThreadLocalFileCollector<'a> {
    fn new(shared: &'a Mutex<LocalFileCollection>) -> Self {
        Self {
            shared,
            local: LocalFileCollection::default(),
        }
    }

    fn push(&mut self, path: &Path, language: &str) {
        self.local.files.push(DiscoveredFile {
            path: path.to_string_lossy().to_string(),
            identity: file_identity(path),
        });
        self.local.languages.insert(language.to_string());
    }
}

impl Drop for ThreadLocalFileCollector<'_> {
    fn drop(&mut self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.files.append(&mut self.local.files);
            shared.languages.append(&mut self.local.languages);
        }
    }
}

#[cfg(unix)]
fn file_identity(path: &Path) -> FileIdentity {
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    fs::metadata(path).map_or_else(
        |_| FileIdentity::default(),
        |metadata| FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        },
    )
}

#[cfg(not(unix))]
fn file_identity(path: &Path) -> FileIdentity {
    FileIdentity {
        path: path.to_string_lossy().to_string(),
    }
}

pub fn collect_files(path: &str, language: Option<&str>) -> FileDiscovery {
    let discovered = Mutex::new(LocalFileCollection::default());
    let walker = ignore::WalkBuilder::new(path)
        .hidden(false)
        .require_git(false)
        .build_parallel();

    walker.run(|| {
        let mut local = ThreadLocalFileCollector::new(&discovered);
        Box::new(move |entry| {
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            let p = entry.path();
            if p.is_file() {
                let detected_language = detect_language(p);
                if let Some(detected_language) = detected_language {
                    let matches_language =
                        language.is_none_or(|expected| detected_language.name == expected);
                    if matches_language {
                        local.push(p, detected_language.name);
                    }
                }
            }
            WalkState::Continue
        })
    });

    let discovered = discovered.into_inner().unwrap_or_default();
    let mut files = discovered.files;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut seen = HashSet::new();
    let files = files
        .into_iter()
        .filter_map(|file| seen.insert(file.identity).then_some(file.path))
        .collect();

    FileDiscovery {
        files,
        languages: discovered.languages,
    }
}

pub fn progress_style(unit: &str, bar_style: &str) -> ProgressStyle {
    let template = format!(
        "{{msg}} [{{bar:40.{bar_style}}}] {{percent_floor}}% | {{pos}}/{{len}} {unit} | ETA {{eta_clamped}}"
    );
    ProgressStyle::with_template(&template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .with_key(
            "percent_floor",
            |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                let len = state.len().unwrap_or(0);
                let pos = state.pos();
                let percent_hundredths = pos.saturating_mul(10_000).checked_div(len).unwrap_or(0);
                let integer = percent_hundredths / 100;
                let fraction = percent_hundredths % 100;
                let _ = write!(w, "{integer:>3}.{fraction:02}");
            },
        )
        .with_key(
            "eta_clamped",
            |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                let len = state.len().unwrap_or(0);
                let pos = state.pos();
                let eta = if len > 0 && pos < len {
                    state.eta().max(std::time::Duration::from_secs(1))
                } else {
                    state.eta()
                };
                let seconds = eta.as_secs();
                let hours = seconds / 3600;
                let minutes = (seconds % 3600) / 60;
                let secs = seconds % 60;
                let _ = write!(w, "{hours:02}:{minutes:02}:{secs:02}");
            },
        )
        .progress_chars("##-")
}

pub fn progress_bar(total: usize, unit: &str, bar_style: &str, message: &str) -> ProgressBar {
    let progress = if std::io::stderr().is_terminal() {
        ProgressBar::with_draw_target(Some(total as u64), ProgressDrawTarget::stderr_with_hz(20))
    } else {
        ProgressBar::hidden()
    };
    progress.set_style(progress_style(unit, bar_style));
    progress.set_message(message.to_string());
    progress
}

pub fn select_most_specific_by_line<C, Bounds>(
    candidates: &[C],
    line: usize,
    bounds: Bounds,
) -> Option<&C>
where
    Bounds: Fn(&C) -> (usize, usize),
{
    let upper_bound = candidates.partition_point(|candidate| bounds(candidate).0 <= line);
    let mut best_index: Option<usize> = None;
    let mut best_start = 0usize;

    for index in (0..upper_bound).rev() {
        let (start_line, end_line) = bounds(&candidates[index]);
        if best_index.is_some() && start_line < best_start {
            break;
        }
        if !(start_line <= line && line <= end_line) {
            continue;
        }

        match best_index {
            None => {
                best_index = Some(index);
                best_start = start_line;
            }
            Some(current_best) => {
                let (best_candidate_start, best_candidate_end) = bounds(&candidates[current_best]);
                let current_span = end_line.saturating_sub(start_line);
                let best_span = best_candidate_end.saturating_sub(best_candidate_start);
                let is_better = start_line > best_candidate_start
                    || (start_line == best_candidate_start
                        && (current_span < best_span
                            || (current_span == best_span && end_line < best_candidate_end)));
                if is_better {
                    best_index = Some(index);
                    best_start = start_line;
                }
            }
        }
    }

    best_index.map(|index| &candidates[index])
}

pub fn relative_path(path: &str, root: &str) -> String {
    let path = Path::new(path);
    let root = Path::new(root);
    path.strip_prefix(root).map_or_else(
        |_| path.to_string_lossy().to_string(),
        |relative| relative.to_string_lossy().to_string(),
    )
}

pub fn resolve_path(path: &str) -> String {
    if let Ok(path) = std::fs::canonicalize(path) {
        path.to_string_lossy().to_string()
    } else {
        let path_ref = Path::new(path);
        if path_ref.is_absolute() {
            path.to_string()
        } else {
            std::env::current_dir().map_or_else(
                |_| path.to_string(),
                |cwd| cwd.join(path).to_string_lossy().to_string(),
            )
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
pub fn success_response(path: &str, searched_files: usize, results: Value) -> Value {
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

pub fn error_response(error: impl std::fmt::Display) -> Value {
    json!({ "error": error.to_string() })
}

pub fn relativize_json_file_paths(value: &mut Value, root: &str) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map.iter_mut() {
                if key == "location" {
                    if let Value::Object(location) = nested {
                        if let Some(Value::String(file)) = location.get_mut("file") {
                            *file = relative_path(file, root);
                        }
                    }
                }
                relativize_json_file_paths(nested, root);
            }
        }
        Value::Array(items) => {
            for item in items {
                relativize_json_file_paths(item, root);
            }
        }
        _ => {}
    }
}
