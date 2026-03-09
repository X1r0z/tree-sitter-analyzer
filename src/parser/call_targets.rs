use std::collections::HashSet;
use std::hash::Hash;

pub(crate) fn split_function_target(function_name: &str) -> (&str, Option<&str>) {
    if function_name.contains('.') {
        let parts: Vec<&str> = function_name.rsplitn(2, '.').collect();
        (parts[0], Some(parts[1]))
    } else {
        (function_name, None)
    }
}

pub(crate) fn extract_instance_attr(object_name: &str) -> Option<&str> {
    for prefix in ["self.", "this.", "cls."] {
        if let Some(rest) = object_name.strip_prefix(prefix) {
            if !rest.is_empty() {
                return rest.split('.').next();
            }
        }
    }
    let candidate = object_name.split('.').next().unwrap_or(object_name);
    (!candidate.is_empty()).then_some(candidate)
}

pub(crate) fn type_matches_class(field_type: Option<&str>, class_name: &str) -> bool {
    let Some(field_type) = field_type else {
        return false;
    };
    field_type
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|token| !token.is_empty() && token == class_name)
}

pub(crate) struct ForwardTargetContext<'a> {
    pub(crate) caller_class_name: Option<&'a str>,
    pub(crate) caller_file: &'a str,
    pub(crate) object_name: Option<&'a str>,
}

pub(crate) fn matches_call_target<FieldMatches, ParamMatches>(
    caller_class_name: Option<&str>,
    object_name: Option<&str>,
    target_class_name: &str,
    mut field_matches: FieldMatches,
    mut param_matches: ParamMatches,
) -> bool
where
    FieldMatches: FnMut(&str, &str) -> bool,
    ParamMatches: FnMut(&str, &str) -> bool,
{
    if object_name == Some(target_class_name) {
        return true;
    }
    if object_name.is_none() {
        return caller_class_name == Some(target_class_name);
    }
    if matches!(object_name, Some("self") | Some("this") | Some("cls")) {
        return caller_class_name == Some(target_class_name);
    }

    let Some(object_name) = object_name else {
        return false;
    };

    if object_name
        .split('(')
        .next()
        .and_then(|head| head.rsplit('.').next())
        .is_some_and(|name| name == target_class_name)
    {
        return true;
    }

    let Some(attr_name) = extract_instance_attr(object_name) else {
        return false;
    };

    field_matches(attr_name, target_class_name) || param_matches(attr_name, target_class_name)
}

pub(crate) fn resolve_forward_targets<
    C,
    K,
    KeyOf,
    ClassNameOf,
    FileOf,
    FieldMatches,
    ParamMatches,
>(
    context: ForwardTargetContext<'_>,
    candidates: &[C],
    key_of: KeyOf,
    class_name_of: ClassNameOf,
    file_of: FileOf,
    mut field_matches: FieldMatches,
    mut param_matches: ParamMatches,
) -> Vec<C>
where
    C: Clone,
    K: Eq + Hash,
    KeyOf: Fn(&C) -> K,
    ClassNameOf: Fn(&C) -> Option<&str>,
    FileOf: Fn(&C) -> &str,
    FieldMatches: FnMut(&str, &str) -> bool,
    ParamMatches: FnMut(&str, &str) -> bool,
{
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    match context.object_name {
        Some("self") | Some("this") | Some("cls") => {
            if let Some(class_name) = context.caller_class_name {
                for candidate in candidates {
                    if class_name_of(candidate) == Some(class_name) {
                        push_unique(&mut results, &mut seen, candidate, &key_of);
                    }
                }
            }
        }
        Some(object_name) => {
            for candidate in candidates {
                if class_name_of(candidate) == Some(object_name) {
                    push_unique(&mut results, &mut seen, candidate, &key_of);
                }
            }

            if let Some(attr_name) = extract_instance_attr(object_name) {
                for candidate in candidates {
                    if let Some(candidate_class_name) = class_name_of(candidate) {
                        if field_matches(attr_name, candidate_class_name)
                            || param_matches(attr_name, candidate_class_name)
                        {
                            push_unique(&mut results, &mut seen, candidate, &key_of);
                        }
                    }
                }
            }
        }
        None => {
            if let Some(class_name) = context.caller_class_name {
                for candidate in candidates {
                    if class_name_of(candidate) == Some(class_name) {
                        push_unique(&mut results, &mut seen, candidate, &key_of);
                    }
                }
            }

            let same_file_globals: Vec<_> = candidates
                .iter()
                .filter(|candidate| {
                    class_name_of(candidate).is_none() && file_of(candidate) == context.caller_file
                })
                .cloned()
                .collect();
            if same_file_globals.is_empty() {
                for candidate in candidates {
                    if class_name_of(candidate).is_none() {
                        push_unique(&mut results, &mut seen, candidate, &key_of);
                    }
                }
            } else {
                for candidate in &same_file_globals {
                    push_unique(&mut results, &mut seen, candidate, &key_of);
                }
            }
        }
    }

    results
}

fn push_unique<C, K, KeyOf>(
    results: &mut Vec<C>,
    seen: &mut HashSet<K>,
    candidate: &C,
    key_of: &KeyOf,
) where
    C: Clone,
    K: Eq + Hash,
    KeyOf: Fn(&C) -> K,
{
    let key = key_of(candidate);
    if seen.insert(key) {
        results.push(candidate.clone());
    }
}
