use std::collections::HashSet;
use std::path::Path;

use regex::Regex;

use crate::languages::get_supported_extensions;

pub fn find_files(path: &str) -> Vec<String> {
    let extensions = get_supported_extensions();
    let ext_set: HashSet<&str> = extensions.iter().copied().collect();

    let mut files = Vec::new();
    let walker = ignore::WalkBuilder::new(path)
        .hidden(false)
        .git_ignore(false)
        .build();

    for entry in walker.flatten() {
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                let dotted = format!(".{}", ext);
                if ext_set.contains(dotted.as_str()) {
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

pub fn rg_search_files(text: &str, path: &str) -> Option<Vec<String>> {
    let rg = which::which("rg").ok()?;
    let extensions = get_supported_extensions();

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

pub fn match_query(name: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    match Regex::new(query) {
        Ok(re) => re.is_match(name),
        Err(_) => name.contains(query),
    }
}

pub fn is_simple_query(query: &str) -> bool {
    !query.is_empty() && !query.contains(|c: char| ".^$*+?{}[]|()\\".contains(c))
}
