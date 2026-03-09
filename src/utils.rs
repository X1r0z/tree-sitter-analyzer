use std::io::IsTerminal;
use std::path::Path;

use crate::languages::{language_extensions, supported_extensions};
use crate::models::{CalleeInfo, CallerInfo};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use serde_json::Value;

pub fn find_files(path: &str, language: Option<&str>) -> Vec<String> {
    let extensions = language
        .and_then(language_extensions)
        .unwrap_or_else(supported_extensions);

    let mut files = Vec::new();
    let walker = ignore::WalkBuilder::new(path)
        .hidden(false)
        .require_git(false)
        .build();

    for entry in walker.flatten() {
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                let dotted = format!(".{}", ext);
                if extensions.contains(&dotted.as_str()) {
                    if let Ok(canonical) = p.canonicalize() {
                        files.push(canonical.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
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

pub fn sort_callers_by_file_line(results: &mut [CallerInfo]) {
    results.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then(left.line.cmp(&right.line))
            .then(left.caller.cmp(&right.caller))
    });
}

pub fn sort_callees_by_file_line(results: &mut [CalleeInfo]) {
    results.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then(left.line.cmp(&right.line))
            .then(left.callee.cmp(&right.callee))
            .then(left.class_name.cmp(&right.class_name))
    });
}

pub fn select_most_specific_by_line<C, I, Bounds>(
    candidates: I,
    line: usize,
    bounds: Bounds,
) -> Option<C>
where
    I: IntoIterator<Item = C>,
    Bounds: Fn(&C) -> (usize, usize),
{
    candidates
        .into_iter()
        .filter(|candidate| {
            let (start_line, end_line) = bounds(candidate);
            start_line <= line && line <= end_line
        })
        .max_by(|left, right| {
            let (left_start, left_end) = bounds(left);
            let (right_start, right_end) = bounds(right);
            let left_span = left_end.saturating_sub(left_start);
            let right_span = right_end.saturating_sub(right_start);

            left_start
                .cmp(&right_start)
                .then_with(|| right_span.cmp(&left_span))
                .then_with(|| right_end.cmp(&left_end))
        })
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
                if key == "file" {
                    if let Some(file) = nested.as_str() {
                        *nested = Value::String(relative_path(file, root));
                    }
                    continue;
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
