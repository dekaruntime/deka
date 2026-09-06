//! Compile a DekaScript graph by exec'ing `dsc` (rfd#38).
//!
//! Returns `Ok(None)` when no dsc binary is configured so callers can fall
//! back to in-process `deka_compile`. Needs dsc >= 0.5.0 (`--self-contained`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

    let input = if entry.starts_with(project_root) {
        project_root
    } else {
        entry.parent().unwrap_or(project_root)
    };

    let output = Command::new(&dsc)
        .args([
            "transpile",
            "--self-contained",
            "--preserve",
            input.to_str().ok_or_else(|| "project path is not UTF-8".to_string())?,
            "--out",
            out.to_str().ok_or_else(|| "cache path is not UTF-8".to_string())?,
        ])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "dsc transpile failed (need dsc >= 0.5.0 for --self-contained):\n{stderr}"
        ));
    }

    let mut modules = HashMap::new();
    collect_js(&out, input, &mut modules)?;
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
        let key = fs::canonicalize(&source).unwrap_or(source);
        modules.insert(key, js);
    }
    Ok(())
}
