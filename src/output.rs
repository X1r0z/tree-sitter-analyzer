use serde_json::Value;

pub fn print_pretty(result: &Value) {
    if let Some(error) = result.get("error").and_then(|e| e.as_str()) {
        eprintln!("Error: {}", error);
        return;
    }

    let path = result.get("path").and_then(|p| p.as_str()).unwrap_or("");
    let count = result.get("count").and_then(|c| c.as_u64()).unwrap_or(0);

    println!("Path: {}", path);
    if let Some(fs) = result.get("files_searched").and_then(|f| f.as_u64()) {
        println!("Files searched: {}", fs);
    }
    println!("Count: {}", count);
    println!("{}", "-".repeat(50));

    if let Some(functions) = result.get("functions").and_then(|f| f.as_array()) {
        for f in functions {
            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let start = f.get("start_line").and_then(|l| l.as_u64()).unwrap_or(0);
            let end = f.get("end_line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = f.get("file").and_then(|fi| fi.as_str()).unwrap_or("");
            let class_name = f.get("class_name").and_then(|c| c.as_str());
            let is_method = f
                .get("is_method")
                .and_then(|m| m.as_bool())
                .unwrap_or(false);

            let class_info = class_name.map(|c| format!(" ({})", c)).unwrap_or_default();
            let method_tag = if is_method { " [method]" } else { "" };
            let loc = format!("L{}-{}", start, end);

            if file.is_empty() {
                println!("  {}{}{} - {}", name, class_info, method_tag, loc);
            } else {
                println!("  {}{}{} - {}:{}", name, class_info, method_tag, file, loc);
            }
            if let Some(body) = f.get("body").and_then(|b| b.as_str()) {
                let truncated = if body.len() > 100 {
                    format!("{}...", &body[..100])
                } else {
                    body.to_string()
                };
                println!("    {}", truncated);
            }
        }
    }

    if let Some(classes) = result.get("classes").and_then(|c| c.as_array()) {
        for c in classes {
            let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let start = c.get("start_line").and_then(|l| l.as_u64()).unwrap_or(0);
            let end = c.get("end_line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = c.get("file").and_then(|fi| fi.as_str()).unwrap_or("");
            let loc = format!("L{}-{}", start, end);

            if file.is_empty() {
                println!("  {} - {}", name, loc);
            } else {
                println!("  {} - {}:{}", name, file, loc);
            }
            if let Some(methods) = c.get("methods").and_then(|m| m.as_array()) {
                if !methods.is_empty() {
                    let method_names: Vec<&str> =
                        methods.iter().filter_map(|m| m.as_str()).collect();
                    println!("    Methods: {}", method_names.join(", "));
                }
            }
            if let Some(fields) = c.get("fields").and_then(|f| f.as_array()) {
                if !fields.is_empty() {
                    let field_names: Vec<&str> = fields.iter().filter_map(|f| f.as_str()).collect();
                    println!("    Fields: {}", field_names.join(", "));
                }
            }
        }
    }

    if let Some(fields) = result.get("fields").and_then(|f| f.as_array()) {
        for f in fields {
            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let line = f.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = f.get("file").and_then(|fi| fi.as_str()).unwrap_or("");
            let type_info = f
                .get("type")
                .and_then(|t| t.as_str())
                .map(|t| format!(" ({})", t))
                .unwrap_or_default();

            if file.is_empty() {
                println!("  {}{} - L{}", name, type_info, line);
            } else {
                println!("  {}{} - {}:L{}", name, type_info, file, line);
            }
        }
    }

    if let Some(imports) = result.get("imports").and_then(|i| i.as_array()) {
        for i in imports {
            let module = i.get("module").and_then(|m| m.as_str()).unwrap_or("");
            let line = i.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = i.get("file").and_then(|fi| fi.as_str()).unwrap_or("");

            if file.is_empty() {
                println!("  {} - L{}", module, line);
            } else {
                println!("  {} - {}:L{}", module, file, line);
            }
        }
    }

    if let Some(variables) = result.get("variables").and_then(|v| v.as_array()) {
        for v in variables {
            let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let line = v.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = v.get("file").and_then(|fi| fi.as_str()).unwrap_or("");
            let scope = v.get("scope").and_then(|s| s.as_str());
            let scope_info = match scope {
                Some(s) => format!(" (scope: {})", s),
                None => " (global)".to_string(),
            };

            if file.is_empty() {
                println!("  {}{} - L{}", name, scope_info, line);
            } else {
                println!("  {}{} - {}:L{}", name, scope_info, file, line);
            }
        }
    }

    if let Some(callers) = result.get("callers").and_then(|c| c.as_array()) {
        let func = result
            .get("function")
            .and_then(|f| f.as_str())
            .unwrap_or("");
        println!("Callers of '{}':", func);
        for c in callers {
            let caller = c.get("caller").and_then(|c| c.as_str()).unwrap_or("");
            let line = c.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = c.get("file").and_then(|fi| fi.as_str()).unwrap_or("");

            if file.is_empty() {
                println!("  {} - L{}", caller, line);
            } else {
                println!("  {} - {}:L{}", caller, file, line);
            }
        }
    }

    if let Some(callees) = result.get("callees").and_then(|c| c.as_array()) {
        let func = result
            .get("function")
            .and_then(|f| f.as_str())
            .unwrap_or("");
        println!("Callees of '{}':", func);
        for c in callees {
            let callee = c.get("callee").and_then(|c| c.as_str()).unwrap_or("");
            let line = c.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = c.get("file").and_then(|fi| fi.as_str()).unwrap_or("");

            if file.is_empty() {
                println!("  {} - L{}", callee, line);
            } else {
                println!("  {} - {}:L{}", callee, file, line);
            }
        }
    }

    if let Some(references) = result.get("references").and_then(|r| r.as_array()) {
        let name = result.get("name").and_then(|n| n.as_str()).unwrap_or("");
        println!("References to '{}':", name);
        for r in references {
            let ref_type = r.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let loc = r.get("location").and_then(|l| l.as_object());
            let file = loc
                .and_then(|l| l.get("file"))
                .and_then(|f| f.as_str())
                .unwrap_or("");
            let line = loc
                .and_then(|l| l.get("start_line"))
                .and_then(|l| l.as_u64())
                .unwrap_or(0);

            if file.is_empty() {
                println!("  {} - L{}", ref_type, line);
            } else {
                println!("  {} - {}:L{}", ref_type, file, line);
            }
        }
    }

    if let Some(strings) = result.get("strings").and_then(|s| s.as_array()) {
        for s in strings {
            let value = s.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let line = s.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = s.get("file").and_then(|fi| fi.as_str()).unwrap_or("");
            let display_value = if value.len() > 50 {
                format!("{}...", &value[..50])
            } else {
                value.to_string()
            };

            if file.is_empty() {
                println!("  {} - L{}", display_value, line);
            } else {
                println!("  {} - {}:L{}", display_value, file, line);
            }
        }
    }

    if let Some(super_classes) = result.get("super_classes").and_then(|s| s.as_array()) {
        let class_name = result
            .get("class_name")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        println!("Super classes of '{}':", class_name);
        for c in super_classes {
            let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let start = c.get("start_line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = c.get("file").and_then(|fi| fi.as_str()).unwrap_or("");

            if file.is_empty() {
                println!("  {} - L{}", name, start);
            } else {
                println!("  {} - {}:L{}", name, file, start);
            }
        }
    }

    if let Some(sub_classes) = result.get("sub_classes").and_then(|s| s.as_array()) {
        let class_name = result
            .get("class_name")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        println!("Sub classes of '{}':", class_name);
        for c in sub_classes {
            let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let start = c.get("start_line").and_then(|l| l.as_u64()).unwrap_or(0);
            let file = c.get("file").and_then(|fi| fi.as_str()).unwrap_or("");

            if file.is_empty() {
                println!("  {} - L{}", name, start);
            } else {
                println!("  {} - {}:L{}", name, file, start);
            }
        }
    }
}
