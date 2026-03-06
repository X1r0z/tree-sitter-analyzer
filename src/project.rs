use std::collections::{HashSet, VecDeque};
use std::path::Path;

use rayon::prelude::*;

use crate::analyzer::CodeAnalyzer;
use crate::cache::TextFilterCache;
use crate::nodes::*;
use crate::utils::{find_files, is_simple_query, rg_search_files, sort_by_file_line, QueryMatcher};

pub struct ProjectAnalyzer {
    pub files: Vec<String>,
    path: String,
    text_filter_cache: TextFilterCache,
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
            text_filter_cache: TextFilterCache::new(),
        })
    }

    pub fn get_functions(&self, query: &str) -> Vec<FunctionInfo> {
        self.collect_functions(query, false)
    }

    pub fn get_functions_with_bodies(&self, query: &str) -> Vec<FunctionInfo> {
        self.collect_functions(query, true)
    }

    fn collect_functions(&self, query: &str, include_body: bool) -> Vec<FunctionInfo> {
        let candidate_files = self.filter_candidates(query);
        if candidate_files.is_empty() {
            return Vec::new();
        }
        let matcher = QueryMatcher::new(query);
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                let funcs = if include_body {
                    analyzer.get_functions_with_bodies()
                } else {
                    analyzer.get_functions()
                };
                if matcher.matches_all() {
                    funcs
                } else {
                    funcs
                        .into_iter()
                        .filter(|func| matcher.is_match(&func.name))
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
        let matcher = QueryMatcher::new(query);
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                let classes = analyzer.get_classes();
                if matcher.matches_all() {
                    classes
                } else {
                    classes
                        .into_iter()
                        .filter(|class| matcher.is_match(&class.name))
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
        let matcher = QueryMatcher::new(query);
        candidate_files
            .par_iter()
            .flat_map(|f| {
                let mut analyzer = match CodeAnalyzer::new(f) {
                    Ok(a) => a,
                    Err(_) => return Vec::new(),
                };
                let imports = analyzer.get_imports();
                if matcher.matches_all() {
                    imports
                } else {
                    imports
                        .into_iter()
                        .filter(|import| matcher.is_match(&import.module))
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

        let mut relevant_files: Vec<String> = Vec::new();
        let mut seen_files: HashSet<String> = HashSet::new();
        for func in &func_defs {
            let fp = &func.location.file;
            if seen_files.insert(fp.clone()) {
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

    pub fn get_all_function_definitions_by_name(
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
                analyzer.get_all_function_definitions_by_name(&fn_name, cn.as_deref())
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
        if let Some(cached) = self.text_filter_cache.get(text) {
            return cached;
        }
        let matched_files: Vec<String> = if let Some(rg_files) =
            rg_search_files(text, &self.path, None)
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
