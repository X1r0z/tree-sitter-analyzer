use std::io::IsTerminal;
use std::path::Path;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use regex::Regex;
use serde_json::Value;

use crate::languages::{language_extensions, supported_extensions};

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

pub(crate) fn progress_style(unit: &str, bar_style: &str) -> ProgressStyle {
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

pub(crate) fn progress_bar(
    total: usize,
    unit: &str,
    bar_style: &str,
    message: &str,
) -> ProgressBar {
    let progress = if std::io::stderr().is_terminal() {
        ProgressBar::with_draw_target(Some(total as u64), ProgressDrawTarget::stderr_with_hz(20))
    } else {
        ProgressBar::hidden()
    };
    progress.set_style(progress_style(unit, bar_style));
    progress.set_message(message.to_string());
    progress
}

pub fn search_files_with_rg(text: &str, path: &str, language: Option<&str>) -> Option<Vec<String>> {
    let rg = which::which("rg").ok()?;
    let extensions = language
        .and_then(language_extensions)
        .unwrap_or_else(supported_extensions);

    let mut cmd = std::process::Command::new(rg);
    cmd.args(["--files-with-matches", "--fixed-strings", "--no-ignore"]);
    for ext in extensions {
        cmd.args(["--glob", &format!("*{}", ext)]);
    }
    cmd.args(["--", text, path]);

    let output = cmd.output().ok()?;
    if !output.status.success() && output.status.code() != Some(1) {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files: Vec<String> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            Path::new(l)
                .canonicalize()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        })
        .collect();
    Some(files)
}

pub fn sort_by_file_line(results: &mut [serde_json::Value]) {
    results.sort_by(|a, b| {
        let fa = a["file"].as_str().unwrap_or("");
        let fb = b["file"].as_str().unwrap_or("");
        let la = a["line"].as_u64().unwrap_or(0);
        let lb = b["line"].as_u64().unwrap_or(0);
        fa.cmp(fb).then(la.cmp(&lb))
    });
}

pub(crate) fn relative_path(path: &str, root: &str) -> String {
    let path = Path::new(path);
    let root = Path::new(root);
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

pub(crate) fn relativize_json_file_paths(value: &mut Value, root: &str) {
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

pub fn is_simple_query(query: &str) -> bool {
    !query.is_empty() && !query.contains(|c: char| ".^$*+?{}[]|()\\".contains(c))
}

pub enum QueryMatcher {
    MatchAll,
    Regex(Regex),
    Contains(String),
}

impl QueryMatcher {
    pub fn new(query: &str) -> Self {
        if query.is_empty() {
            return Self::MatchAll;
        }
        match Regex::new(query) {
            Ok(re) => Self::Regex(re),
            Err(_) => Self::Contains(query.to_string()),
        }
    }

    pub fn is_match(&self, name: &str) -> bool {
        match self {
            Self::MatchAll => true,
            Self::Regex(re) => re.is_match(name),
            Self::Contains(text) => name.contains(text),
        }
    }

    pub fn matches_all(&self) -> bool {
        matches!(self, Self::MatchAll)
    }
}
