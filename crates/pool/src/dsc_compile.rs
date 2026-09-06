//! Compile a DekaScript graph by exec'ing `dsc` (rfd#38).
//!
//! Returns `Ok(None)` when no dsc binary is configured so callers can fall
//! back to in-process `deka_compile`. Needs dsc >= 0.5.0 (`--self-contained`).
//!
//! dsc 0.5.1 preserve-mode rewrites relative `.ds` imports to `.js`. The
//! isolate loader resolves `.ds` paths from memory, so those rewrites ENOENT.
//! Restore them here until a dsc that keeps specifiers under `--self-contained`
//! is the published latest.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use runtime_core::DEKA_VALIDATION_ERROR_MARKER;

pub fn compile_graph(
    project_root: &Path,
    entry: &Path,
) -> Result<Option<HashMap<PathBuf, String>>, String> {
    let Some(dsc) = runtime_core::dsc::find_dsc()? else {
        return Ok(None);
    };
    let out = project_root.join(".cache").join("dsc-modules");
    if out.exists() {
        let _ = fs::remove_dir_all(&out);
    }
    fs::create_dir_all(&out)
        .map_err(|err| format!("failed to create {}: {err}", out.display()))?;

    let entry = if entry.is_absolute() {
        entry.to_path_buf()
    } else {
        project_root.join(entry)
    };

    let output = Command::new(&dsc)
        .current_dir(project_root)
        .env("DEKA_MODULE_ROOT", project_root)
        .args([
            "transpile",
            "--self-contained",
            entry
                .to_str()
                .ok_or_else(|| "entry path is not UTF-8".to_string())?,
            "--out",
            out.to_str().ok_or_else(|| "cache path is not UTF-8".to_string())?,
        ])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{DEKA_VALIDATION_ERROR_MARKER}dsc transpile failed (need dsc >= 0.5.0 for --self-contained):\n{stderr}"
        ));
    }

    let mut modules = HashMap::new();
    collect_js(&out, project_root, &mut modules)?;
    if modules.is_empty() {
        return Err("dsc transpile wrote no modules".to_string());
    }
    Ok(Some(modules))
}

fn collect_js(
    out_root: &Path,
    source_root: &Path,
    modules: &mut HashMap<PathBuf, String>,
) -> Result<(), String> {
    let entries = fs::read_dir(out_root)
        .map_err(|err| format!("failed to read {}: {err}", out_root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read {}: {err}", out_root.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_js(&path, source_root, modules)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("js") {
            continue;
        }
        let rel = path
            .strip_prefix(out_root)
            .map_err(|_| "dsc output escaped the cache directory".to_string())?;
        let ds = source_root.join(rel).with_extension("ds");
        let dsx = source_root.join(rel).with_extension("dsx");
        let source = if ds.is_file() {
            ds
        } else if dsx.is_file() {
            dsx
        } else {
            continue;
        };
        let js = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let js = restore_isolate_specifiers(js, &source);
        let key = fs::canonicalize(&source).unwrap_or(source);
        modules.insert(key, js);
    }
    Ok(())
}

/// dsc preserve-mode writes `from "./foo.js"`. Isolate resolve looks for
/// `foo.ds` / `foo.dsx`. Put the original specifier back when that source
/// sits next to the module we just compiled.
fn restore_isolate_specifiers(mut js: String, source_file: &Path) -> String {
    let Some(dir) = source_file.parent() else {
        return js;
    };
    for quote in ['\'', '"'] {
        let needle = format!("from {quote}");
        let mut cursor = 0;
        while let Some(found) = js[cursor..].find(&needle) {
            let start = cursor + found + needle.len();
            let Some(rel_end) = js[start..].find(quote) else {
                break;
            };
            let end = start + rel_end;
            let specifier = js[start..end].to_string();
            if !(specifier.starts_with("./") || specifier.starts_with("../"))
                || !specifier.ends_with(".js")
            {
                cursor = end + 1;
                continue;
            }
            let peer = dir.join(&specifier);
            let ds = peer.with_extension("ds");
            let dsx = peer.with_extension("dsx");
            let new_ext = if ds.is_file() {
                "ds"
            } else if dsx.is_file() {
                "dsx"
            } else {
                cursor = end + 1;
                continue;
            };
            js.replace_range(end - 2..end, new_ext);
            cursor = end - 2 + new_ext.len();
        }
    }
    js
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_js_imports_to_ds_when_source_exists() {
        let tmp = std::env::temp_dir().join(format!("dsc-restore-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("math.ds"), "export const n = 1\n").unwrap();
        let main = tmp.join("main.ds");
        fs::write(&main, "import { n } from \"./math.ds\"\n").unwrap();
        let js = "import { n } from \"./math.js\";\n".to_string();
        let out = restore_isolate_specifiers(js, &main);
        let _ = fs::remove_dir_all(&tmp);
        assert!(out.contains("./math.ds"), "{out}");
        assert!(!out.contains("./math.js"), "{out}");
    }
}
