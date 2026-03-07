pub(super) fn split_function_target(function_name: &str) -> (&str, Option<&str>) {
    if function_name.contains('.') {
        let parts: Vec<&str> = function_name.rsplitn(2, '.').collect();
        (parts[0], Some(parts[1]))
    } else {
        (function_name, None)
    }
}

pub(super) fn repeat_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn extract_instance_attr(object_name: &str) -> Option<String> {
    for prefix in ["self.", "this.", "cls."] {
        if let Some(rest) = object_name.strip_prefix(prefix) {
            if !rest.is_empty() {
                return Some(rest.split('.').next().unwrap_or(rest).to_string());
            }
        }
    }
    None
}

pub(super) fn type_matches_class(field_type: Option<&str>, class_name: &str) -> bool {
    let Some(field_type) = field_type else {
        return false;
    };
    field_type
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|token| !token.is_empty() && token == class_name)
}
