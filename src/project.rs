use std::collections::{HashSet, VecDeque};
use std::path::Path;

use rayon::prelude::*;

use crate::analyzer::CodeAnalyzer;
use crate::nodes::*;
use crate::utils::{find_files, is_simple_query, match_query, rg_search_files, sort_by_file_line};

pub struct ProjectAnalyzer {
    pub files: Vec<String>,
    path: String,
}

impl ProjectAnalyzer {
    pub fn new_with_language(path: &str, language: Option<&str>) -> anyhow::Result<Self> {
        let p = Path::new(path);
        if !p.exists() {
            anyhow::bail!("Path not found: {}", path);
        }
        if !p.is_dir() {
            anyhow::bail!("Path must be a directory: {}", path);
        }
        let files = find_files(path, language);
        Ok(Self {
            files,
            path: path.to_string(),
        })
    }

    pub fn get_functions(&self, query: &str) -> Vec<FunctionInfo> {
        let candidate_files = self.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let q = query.to_string();
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                let funcs = analyzer.get_functions();
                if q.is_empty() {
                    funcs
                } else {
                    funcs
                        .into_iter()
                        .filter(|func| match_query(&func.name, &q))
                        .collect()
                }
            })
            .collect()
    }

    pub fn get_classes(&self, query: &str) -> Vec<ClassInfo> {
        let candidate_files = self.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let q = query.to_string();
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                let classes = analyzer.get_classes();
                if q.is_empty() {
                    classes
                } else {
                    classes
                        .into_iter()
                        .filter(|class| match_query(&class.name, &q))
                        .collect()
                }
            })
            .collect()
    }

    pub fn get_fields(&self, class_name: &str) -> Vec<FieldInfo> {
        let candidate_files = self.filter_by_text(class_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let cn = class_name.to_string();
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                analyzer.get_fields(&cn)
            })
            .collect()
    }

    pub fn get_imports(&self, query: &str) -> Vec<ImportInfo> {
        let candidate_files = self.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let q = query.to_string();
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                let imports = analyzer.get_imports();
                if q.is_empty() {
                    imports
                } else {
                    imports
                        .into_iter()
                        .filter(|import| match_query(&import.module, &q))
                        .collect()
                }
            })
            .collect()
    }
    pub fn get_callers(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let candidate_files = self.filter_by_text(function_name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let fn_name = function_name.to_string();
        let cn = class_name.map(|s| s.to_string());
        let mut results: Vec<serde_json::Value> = candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                analyzer
                    .get_function_callers(&fn_name, cn.as_deref())
                    .into_iter()
                    .map(|(caller, line)| {
                        serde_json::json!({
                            "caller": caller,
                            "line": line,
                            "file": f,
                            "target_class": cn.as_deref(),
                        })
                    })
                    .collect()
            })
            .collect();
        sort_by_file_line(&mut results);
        results
    }

    pub fn get_callees(
        &self,
        function_name: &str,
        class_name: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let func_defs = self.get_all_functions_by_name(function_name, class_name);
        let files_set: HashSet<&str> = self.files.iter().map(|s| s.as_str()).collect();

        let mut relevant_files: Vec<String> = Vec::new();
        let mut seen_files: HashSet<String> = HashSet::new();
        for func in &func_defs {
            let fp = &func.location.file;
            if files_set.contains(fp.as_str()) && seen_files.insert(fp.clone()) {
                relevant_files.push(fp.clone());
            }
        }
        if relevant_files.is_empty() {
            relevant_files = self.filter_by_text(function_name);
        }
        if relevant_files.is_empty() {
            return Vec::new();
        }

        let fn_name = function_name.to_string();
        let cn = class_name.map(|s| s.to_string());
        let mut results: Vec<serde_json::Value> = relevant_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                analyzer
                    .get_function_callees(&fn_name, cn.as_deref())
                    .into_iter()
                    .map(|(callee, line, callee_class)| {
                        serde_json::json!({
                            "callee": callee,
                            "line": line,
                            "file": f,
                            "class_name": callee_class,
                        })
                    })
                    .collect()
            })
            .collect();
        sort_by_file_line(&mut results);
        results
    }

    pub fn get_all_functions_by_name(
        &self,
        name: &str,
        class_name: Option<&str>,
    ) -> Vec<FunctionInfo> {
        let candidate_files = self.filter_by_text(name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let fn_name = name.to_string();
        let cn = class_name.map(|s| s.to_string());
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                analyzer.get_all_functions_by_name(&fn_name, cn.as_deref())
            })
            .collect()
    }

    pub fn find_symbols(&self, name: &str) -> Vec<serde_json::Value> {
        let candidate_files = self.filter_by_text(name);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let symbol = name.to_string();
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                analyzer.find_symbols(&symbol)
            })
            .collect()
    }

    pub fn get_super_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        let target = self.find_class_by_name(class_name);
        let target = match target {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut result = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(class_name.to_string());
        let mut queue: VecDeque<ClassInfo> = VecDeque::new();
        queue.push_back(target);

        while let Some(current) = queue.pop_front() {
            for parent_name in &current.super_classes {
                if visited.contains(parent_name) {
                    continue;
                }
                visited.insert(parent_name.clone());
                let candidate_files = self.filter_by_text(parent_name);
                let pn = parent_name.clone();
                let found: Vec<ClassInfo> = candidate_files
                    .par_iter()
                    .flat_map(|f| {
                        let mut analyzer = match CodeAnalyzer::new(f) {
                            Ok(a) => a,
                            Err(_) => return Vec::new(),
                        };
                        analyzer
                            .get_class_by_name(&pn)
                            .into_iter()
                            .collect::<Vec<_>>()
                    })
                    .collect();
                if let Some(parent) = found.into_iter().next() {
                    result.push(parent.clone());
                    queue.push_back(parent);
                }
            }
        }
        result
    }

    pub fn get_sub_classes(&self, class_name: &str) -> Vec<ClassInfo> {
        let mut result = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(class_name.to_string());
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(class_name.to_string());

        while let Some(current_parent) = queue.pop_front() {
            let candidate_files = self.filter_by_text(&current_parent);
            let cp = current_parent.clone();
            let found: Vec<ClassInfo> = candidate_files
                .par_iter()
                .flat_map(|f| {
                    let mut analyzer = match CodeAnalyzer::new(f) {
                        Ok(a) => a,
                        Err(_) => return Vec::new(),
                    };
                    analyzer
                        .get_classes()
                        .into_iter()
                        .filter(|cls| cls.super_classes.contains(&cp))
                        .collect::<Vec<_>>()
                })
                .collect();

            for cls in found {
                if visited.insert(cls.name.clone()) {
                    queue.push_back(cls.name.clone());
                    result.push(cls);
                }
            }
        }
        result
    }

    fn find_class_by_name(&self, class_name: &str) -> Option<ClassInfo> {
        let candidate_files = self.filter_by_text(class_name);
        for f in &candidate_files {
            let mut analyzer = match CodeAnalyzer::new(f) {
                Ok(a) => a,
                Err(_) => continue,
            };
            if let Some(cls) = analyzer.get_class_by_name(class_name) {
                return Some(cls);
            }
        }
        None
    }

    fn filter_candidates(&self, query: &str) -> Vec<String> {
        if !query.is_empty() && is_simple_query(query) {
            self.filter_by_text(query)
        } else {
            self.files.clone()
        }
    }

    fn filter_by_text(&self, text: &str) -> Vec<String> {
        if text.is_empty() {
            return self.files.clone();
        }
        if let Some(rg_files) = rg_search_files(text, &self.path, None) {
            let rg_set: HashSet<String> = rg_files.into_iter().collect();
            self.files
                .iter()
                .filter(|f| rg_set.contains(f.as_str()))
                .cloned()
                .collect()
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
        }
    }
}
