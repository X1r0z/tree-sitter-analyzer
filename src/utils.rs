use std::io::IsTerminal;
use std::path::Path;
use std::sync::Mutex;

use crate::languages::{language_extensions, supported_extensions};
use crate::models::{CalleeInfo, CallerInfo};
use ignore::WalkState;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use serde_json::Value;

pub fn find_files(path: &str, language: Option<&str>) -> Vec<String> {
    let extensions = language
        .and_then(language_extensions)
        .unwrap_or_else(supported_extensions);

    let files = Mutex::new(Vec::new());
    let walker = ignore::WalkBuilder::new(path)
        .hidden(false)
        .require_git(false)
        .build_parallel();

    walker.run(|| {
        let files = &files;
        let extensions = &extensions;
        Box::new(move |entry| {
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            let p = entry.path();
            if p.is_file() {
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    let dotted = format!(".{}", ext);
                    if extensions.contains(&dotted.as_str()) {
                        if let Ok(canonical) = p.canonicalize() {
                            if let Ok(mut matched_files) = files.lock() {
                                matched_files.push(canonical.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
            WalkState::Continue
        })
    });

    let mut files = files.into_inner().unwrap_or_default();
    files.sort();
    files.dedup();
    files
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
                let percent_hundredths = if len == 0 {
                    0
                } else {
                    pos.saturating_mul(10_000) / len
                };
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

pub fn print_warning(message: &str) {
    if std::io::stderr().is_terminal() {
        eprintln!("\x1b[33mwarning:\x1b[0m {message}");
    } else {
        eprintln!("warning: {message}");
    }
}

pub fn sort_callers_by_file_line(results: &mut [CallerInfo]) {
    results.sort_by(|left, right| {
        left.location
            .file
            .cmp(&right.location.file)
            .then(left.location.start_line.cmp(&right.location.start_line))
            .then(left.caller.cmp(&right.caller))
    });
}

pub fn sort_callees_by_file_line(results: &mut [CalleeInfo]) {
    results.sort_by(|left, right| {
        left.location
            .file
            .cmp(&right.location.file)
            .then(left.location.start_line.cmp(&right.location.start_line))
            .then(left.callee.cmp(&right.callee))
    });
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
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

pub fn relativize_json_file_paths(value: &mut Value, root: &str) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map.iter_mut() {
                if key == "location" {
                    if let Value::Object(location) = nested {
                        if let Some(Value::String(path)) = location.get_mut("path") {
                            *path = relative_path(path, root);
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
