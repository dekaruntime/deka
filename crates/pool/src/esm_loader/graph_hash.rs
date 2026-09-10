//! Module-graph hashing for cache invalidation (deka#743's source graph).
//!
//! Walks the DekaScript import graph starting at the entry and hashes every
//! reachable source file, so a content change anywhere in the graph changes
//! the digest even when the entry file itself is untouched.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::resolver::{parse_module_imports, resolve_import_path, resolve_project_root};

pub fn hash_module_graph(entry_path: &Path) -> Result<u64, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let project_root = resolve_project_root(entry_path)?;
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<PathBuf> = if entry_path.is_dir() {
        collect_app_route_files(entry_path)?
    } else {
        vec![entry_path.to_path_buf()]
    };
    let mut hasher = DefaultHasher::new();

    while let Some(path) = stack.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        source.hash(&mut hasher);

        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if ext == "ds" || ext == "dsx" {
            let imports = parse_module_imports(&source);
            for spec in imports {
                if let Some(resolved) = resolve_import_path(&project_root, &path, spec.trim()) {
                    stack.push(resolved);
                }
            }
        }
    }

    Ok(hasher.finish())
}

pub(crate) fn collect_app_route_files(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let app_dir = project_root.join("app");
    if !app_dir.is_dir() {
        return Err(format!(
            "app directory missing for app-mode entry: {}",
            app_dir.display()
        ));
    }
    collect_deka_source_files_recursive(&app_dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_deka_source_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read dir entry: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_deka_source_files_recursive(&path, out)?;
            continue;
        }
        let is_deka_source = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("ds") || ext.eq_ignore_ascii_case("dsx"))
            .unwrap_or(false);
        if is_deka_source {
            out.push(path);
        }
    }
    Ok(())
}
