use rayon::prelude::*;

/// Run a function on each file in parallel and collect results.
/// Each invocation returns a Vec of items; all results are flattened.
pub fn run_parallel<T, F>(files: &[String], func: F) -> Vec<T>
where
    T: Send,
    F: Fn(&str) -> Vec<T> + Sync,
{
    files.par_iter().flat_map(|f| func(f)).collect()
}

/// Sort JSON results by file and line.
pub fn sort_by_file_line(results: &mut [serde_json::Value]) {
    results.sort_by(|a, b| {
        let fa = a["file"].as_str().unwrap_or("");
        let fb = b["file"].as_str().unwrap_or("");
        let la = a["line"].as_u64().unwrap_or(0);
        let lb = b["line"].as_u64().unwrap_or(0);
        fa.cmp(fb).then(la.cmp(&lb))
    });
}
