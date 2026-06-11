use std::collections::HashSet;

use regex_syntax::hir::{Hir, HirKind};

const MIN_FTS_LITERAL_LEN: usize = 3;
const MAX_FTS_LITERALS: usize = 8;

pub(super) struct RegexPrefilter {
    pub(super) match_all: bool,
    pub(super) fts_match_query: Option<String>,
}

impl RegexPrefilter {
    pub(super) fn new(pattern: &str) -> Self {
        if pattern.is_empty() {
            return Self {
                match_all: true,
                fts_match_query: None,
            };
        }

        let mandatory_literals = regex_syntax::parse(pattern)
            .ok()
            .map(|hir| normalize_literals(extract_mandatory_literals(&hir)))
            .unwrap_or_default();

        let fts_literals = build_fts_literals(&mandatory_literals);
        let fts_match_query =
            (!fts_literals.is_empty()).then(|| build_fts_match_query(&fts_literals));

        Self {
            match_all: false,
            fts_match_query,
        }
    }
}

fn extract_mandatory_literals(hir: &Hir) -> Vec<String> {
    match hir.kind() {
        HirKind::Empty | HirKind::Class(_) | HirKind::Look(_) => Vec::new(),
        HirKind::Literal(literal) => std::str::from_utf8(&literal.0)
            .ok()
            .filter(|text| !text.is_empty())
            .map(|text| vec![text.to_string()])
            .unwrap_or_default(),
        HirKind::Capture(capture) => extract_mandatory_literals(&capture.sub),
        HirKind::Repetition(repetition) => {
            if repetition.min == 0 {
                Vec::new()
            } else {
                extract_mandatory_literals(&repetition.sub)
            }
        }
        HirKind::Concat(parts) => normalize_literals(
            parts
                .iter()
                .flat_map(extract_mandatory_literals)
                .collect::<Vec<_>>(),
        ),
        HirKind::Alternation(parts) => {
            let mut iter = parts.iter();
            let Some(first) = iter.next() else {
                return Vec::new();
            };
            let mut common = extract_mandatory_literals(first);
            for part in iter {
                common = intersect_literals(&common, &extract_mandatory_literals(part));
                if common.is_empty() {
                    break;
                }
            }
            normalize_literals(common)
        }
    }
}

fn intersect_literals(left: &[String], right: &[String]) -> Vec<String> {
    if left.is_empty() || right.is_empty() {
        return Vec::new();
    }

    let mut substrings = HashSet::new();
    for literal in left {
        substrings.extend(all_substrings(literal));
    }

    let mut result = Vec::new();
    for candidate in substrings {
        if right.iter().any(|literal| literal.contains(&candidate)) {
            result.push(candidate);
        }
    }
    result
}

fn all_substrings(text: &str) -> HashSet<String> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut substrings = HashSet::new();
    if chars.is_empty() {
        return substrings;
    }

    let mut boundaries: Vec<usize> = chars.iter().map(|(idx, _)| *idx).collect();
    boundaries.push(text.len());

    for start in 0..chars.len() {
        for end in (start + 1)..=chars.len() {
            let substring = &text[boundaries[start]..boundaries[end]];
            if !substring.is_empty() {
                substrings.insert(substring.to_string());
            }
        }
    }
    substrings
}

fn normalize_literals(literals: Vec<String>) -> Vec<String> {
    let mut literals: Vec<String> = literals
        .into_iter()
        .filter(|literal| !literal.is_empty())
        .collect();
    literals.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    literals.dedup();

    let mut result = Vec::new();
    for literal in literals {
        if result
            .iter()
            .any(|existing: &String| existing.contains(&literal))
        {
            continue;
        }
        result.push(literal);
    }
    result
}

fn build_fts_literals(literals: &[String]) -> Vec<String> {
    literals
        .iter()
        .filter(|literal| literal.chars().count() >= MIN_FTS_LITERAL_LEN)
        .take(MAX_FTS_LITERALS)
        .cloned()
        .collect()
}

fn build_fts_match_query(literals: &[String]) -> String {
    literals
        .iter()
        .map(|literal| format!("\"{}\"", literal.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}
