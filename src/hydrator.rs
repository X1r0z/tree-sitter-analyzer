use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use rayon::prelude::*;

use crate::extractor::CodeExtractor;
use crate::models::{FunctionInfo, FunctionKey, Location, RefInfo, RefKey};

pub(crate) fn hydrate_function_bodies(candidates: Vec<FunctionInfo>) -> Vec<FunctionInfo> {
    hydrate_candidates(
        candidates,
        |candidate: &FunctionInfo| FunctionKey::from(candidate),
        |candidate| candidate.location.file.as_str(),
        |extractor, expected| {
            expected
                .iter()
                .flat_map(|key| {
                    extractor
                        .collect_function_definitions(&key.name, key.class_name.as_deref())
                        .into_iter()
                        .filter(|function| FunctionKey::from(function) == *key)
                })
                .collect()
        },
    )
}

pub(crate) fn hydrate_ref_contexts(candidates: Vec<RefInfo>) -> Vec<RefInfo> {
    hydrate_candidates(
        candidates,
        |candidate: &RefInfo| RefKey::from(candidate),
        |candidate| candidate.location.file.as_str(),
        |extractor, expected| {
            let expected_refs: Vec<_> = expected
                .iter()
                .map(|key| RefInfo {
                    name: key.name.clone(),
                    node_type: key.node_type.clone(),
                    location: Location {
                        file: key.file.clone(),
                        start_line: key.start_line,
                        end_line: key.end_line,
                    },
                    start_column: key.start_column,
                    end_column: key.end_column,
                    context: String::new(),
                })
                .collect();
            extractor.hydrate_refs(&expected_refs)
        },
    )
}

fn hydrate_candidates<K, Candidate, KeyOf, FileOf, Resolve>(
    candidates: Vec<Candidate>,
    key_of: KeyOf,
    file_of: FileOf,
    resolve: Resolve,
) -> Vec<Candidate>
where
    K: Clone + Eq + Hash + Send + Sync,
    Candidate: Clone + Send + Sync,
    KeyOf: for<'a> Fn(&'a Candidate) -> K + Sync,
    FileOf: for<'a> Fn(&'a Candidate) -> &'a str,
    Resolve: Fn(&mut CodeExtractor, &HashSet<K>) -> Vec<Candidate> + Sync + Send,
{
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut expected_keys_by_file: HashMap<String, HashSet<K>> = HashMap::new();
    for candidate in &candidates {
        expected_keys_by_file
            .entry(file_of(candidate).to_string())
            .or_default()
            .insert(key_of(candidate));
    }

    let expected_keys_by_file: Vec<_> = expected_keys_by_file.into_iter().collect();
    let resolved: HashMap<K, Candidate> = expected_keys_by_file
        .par_iter()
        .flat_map(|(file, expected)| {
            let Ok(mut extractor) = CodeExtractor::new(file) else {
                return Vec::new();
            };
            resolve(&mut extractor, expected)
                .into_iter()
                .map(|candidate| (key_of(&candidate), candidate))
                .collect::<Vec<_>>()
        })
        .collect();

    candidates
        .into_iter()
        .map(|candidate| {
            resolved
                .get(&key_of(&candidate))
                .cloned()
                .unwrap_or(candidate)
        })
        .collect()
}
