use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rayon::prelude::*;
use regex::Regex;

use crate::languages::{language_extensions, supported_extensions};

pub(crate) struct FileSearch {
    files: Vec<String>,
    path: String,
    text_filter_cache: TextFilterCache,
}

impl FileSearch {
    pub(crate) fn new(path: &str, files: Vec<String>) -> Self {
        Self {
            files,
            path: path.to_string(),
            text_filter_cache: TextFilterCache::new(),
        }
    }

    pub(crate) fn files(&self) -> &[String] {
        &self.files
    }

    pub(crate) fn filter_candidates(&self, query: &str) -> Vec<String> {
        if !query.is_empty() && is_simple_query(query) {
            self.filter_by_text(query)
        } else {
            self.files.clone()
        }
    }

    pub(crate) fn filter_by_text(&self, text: &str) -> Vec<String> {
        if text.is_empty() {
            return self.files.clone();
        }
        if let Some(cached) = self.text_filter_cache.get(text) {
            return cached;
        }
        let matched_files: Vec<String> = if let Some(rg_files) =
            search_files_with_rg(text, &self.path, None)
        {
            let mut rg_files = rg_files;
            rg_files.sort();
            rg_files.dedup();

            let mut matched = Vec::new();
            let mut files_idx = 0usize;
            let mut rg_idx = 0usize;
            while files_idx < self.files.len() && rg_idx < rg_files.len() {
                match self.files[files_idx].cmp(&rg_files[rg_idx]) {
                    std::cmp::Ordering::Less => files_idx += 1,
                    std::cmp::Ordering::Greater => rg_idx += 1,
                    std::cmp::Ordering::Equal => {
                        matched.push(self.files[files_idx].clone());
                        files_idx += 1;
                        rg_idx += 1;
                    }
                }
            }
            matched
        } else {
            self.files
                .par_iter()
                .filter(|f| {
                    std::fs::read(f)
                        .map(|content| content.windows(text.len()).any(|w| w == text.as_bytes()))
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        };
        self.text_filter_cache.insert(text, matched_files.clone());
        matched_files
    }
}

fn search_files_with_rg(text: &str, path: &str, language: Option<&str>) -> Option<Vec<String>> {
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

pub(crate) fn is_simple_query(query: &str) -> bool {
    !query.is_empty() && !query.contains(|c: char| ".^$*+?{}[]|()\\".contains(c))
}

pub(crate) enum QueryMatcher {
    MatchAll,
    Regex(Regex),
    Contains(String),
}

impl QueryMatcher {
    pub(crate) fn new(query: &str) -> Self {
        if query.is_empty() {
            return Self::MatchAll;
        }
        match Regex::new(query) {
            Ok(re) => Self::Regex(re),
            Err(_) => Self::Contains(query.to_string()),
        }
    }

    pub(crate) fn is_match(&self, name: &str) -> bool {
        match self {
            Self::MatchAll => true,
            Self::Regex(re) => re.is_match(name),
            Self::Contains(text) => name.contains(text),
        }
    }

    pub(crate) fn matches_all(&self) -> bool {
        matches!(self, Self::MatchAll)
    }
}

struct TextFilterCache {
    matches_by_text: Mutex<HashMap<String, Vec<String>>>,
}

impl TextFilterCache {
    fn new() -> Self {
        Self {
            matches_by_text: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, text: &str) -> Option<Vec<String>> {
        self.matches_by_text
            .lock()
            .ok()
            .and_then(|cache| cache.get(text).cloned())
    }

    fn insert(&self, text: &str, matched_files: Vec<String>) {
        if let Ok(mut cache) = self.matches_by_text.lock() {
            cache.insert(text.to_string(), matched_files);
        }
    }
}
