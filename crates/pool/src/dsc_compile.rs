//! Compile a DekaScript graph by exec'ing `dsc` (rfd#38).
//!
//! Isolate compile does not fall back to an in-process compiler. Needs
//! dsc >= 0.5.0 (`--self-contained`). Set `DEKA_DSC`, ship `dsc` next to
//! `deka`, or put it on `PATH`.
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
) -> Result<HashMap<PathBuf, String>, String> {
    let dsc = compiler::dsc::find_dsc()?.ok_or_else(|| {
        format!(
            "{DEKA_VALIDATION_ERROR_MARKER}dsc is required to compile DekaScript in the isolate. Set DEKA_DSC, install dsc next to deka, or put dsc on PATH."
        )
    })?;
    compile_graph_with_dsc(project_root, entry, &dsc)
}

/// Compile a graph with a compiler selected by the caller. Build owns its
/// compiler selection at the CLI boundary; runtime/pool do not read it from
/// the ambient process environment.
pub fn compile_graph_with_dsc(
    project_root: &Path,
    entry: &Path,
    dsc: &Path,
) -> Result<HashMap<PathBuf, String>, String> {

    // RFD 21 (deka#754): validate every DekaScript source in the tree against
    // the closed deka.* catalog before dsc runs, lowering `safe { deka.* }`
    // sites to the verbatim call. A tree containing lowered sources compiles
    // against a hardlinked mirror under `.cache` so user files are never
    // modified; dsc diagnostics are mapped back to real source paths.
    let prepared = crate::deka_catalog_stage::prepare_compile_root(project_root)?;
    let (compile_root, compile_entry) = match &prepared {
        crate::deka_catalog_stage::PreparedRoot::InPlace => {
            let entry = if entry.is_absolute() {
                entry.to_path_buf()
            } else {
                project_root.join(entry)
            };
            (project_root.to_path_buf(), entry)
        }
        crate::deka_catalog_stage::PreparedRoot::Staged(guard) => {
            let entry = if entry.is_absolute() {
                entry.to_path_buf()
            } else {
                project_root.join(entry)
            };
            (guard.root().to_path_buf(), guard.map_path(&entry))
        }
    };

    let out = compile_root.join(".cache").join("dsc-modules");
    if out.exists() {
        let _ = fs::remove_dir_all(&out);
    }
    fs::create_dir_all(&out)
        .map_err(|err| format!("failed to create {}: {err}", out.display()))?;

    let output = Command::new(&dsc)
        .current_dir(&compile_root)
        .env("DEKA_MODULE_ROOT", &compile_root)
        .args([
            "transpile",
            "--self-contained",
            compile_entry
                .to_str()
                .ok_or_else(|| "entry path is not UTF-8".to_string())?,
            "--out",
            out.to_str().ok_or_else(|| "cache path is not UTF-8".to_string())?,
        ])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if !output.status.success() {
        // Surface dsc's diagnostics verbatim. An earlier version prefixed a
        // "need dsc >= 0.5.0 for --self-contained" guess, which was stale (every
        // supported dsc satisfies it) and — because it occupied the first line —
        // was what downstream consumers parsed as *the* diagnostic, masking the
        // real one. The conformance harness recorded that guess for 317 tests
        // and dropped 388 genuine diagnostics as a result (deka#739). If a
        // diagnosis is ever added back here it must go after dsc's own output,
        // never in front of it.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let detail = if detail.is_empty() {
            format!("dsc transpile exited {}", output.status)
        } else {
            detail.to_string()
        };
        // A staged compile names mirror paths; report the user's real paths.
        let detail = match &prepared {
            crate::deka_catalog_stage::PreparedRoot::Staged(guard) => guard.remap_diagnostic(&detail),
            crate::deka_catalog_stage::PreparedRoot::InPlace => detail,
        };
        return Err(format!("{DEKA_VALIDATION_ERROR_MARKER}{detail}"));
    }

    let mut modules = HashMap::new();
    collect_js(&out, &compile_root, &mut modules)?;
    if modules.is_empty() {
        return Err("dsc transpile wrote no modules".to_string());
    }
    // Map staged canonical paths back to the user's source paths so the
    // loader's lookups (which key on real .ds paths) hit.
    if let crate::deka_catalog_stage::PreparedRoot::Staged(guard) = &prepared {
        modules = modules
            .into_iter()
            .map(|(key, js)| {
                let mapped = guard.unmap_path(&key);
                let mapped = fs::canonicalize(&mapped).unwrap_or(mapped);
                (mapped, js)
            })
            .collect();
    }
    Ok(modules)
}

/// JS for `source` from a graph dump, trying canonical and macOS `/var`
/// vs `/private/var` aliases so dump keys match isolate loads.
pub fn lookup_js<'a>(
    modules: &'a HashMap<PathBuf, String>,
    source: &Path,
) -> Option<&'a String> {
    let key = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    if let Some(js) = modules.get(&key).or_else(|| modules.get(source)) {
        return Some(js);
    }
    let text = key.to_string_lossy();
    if let Some(rest) = text.strip_prefix("/private") {
        modules.get(Path::new(rest))
    } else {
        modules.get(&PathBuf::from(format!("/private{text}")))
    }
}

/// Compile one `.ds` that was not in the consumer graph (linked packages
/// live outside the project root, so `--self-contained` on the consumer
/// does not dump them).
pub fn compile_file(source: &Path) -> Result<String, String> {
    let root = source.parent().unwrap_or(source);
    let modules = compile_graph(root, source)?;
    compiled_file(&modules, source)
}

/// Compile one linked-package source with the compiler selected by the caller.
pub fn compile_file_with_dsc(source: &Path, dsc: &Path) -> Result<String, String> {
    let root = source.parent().unwrap_or(source);
    let modules = compile_graph_with_dsc(root, source, dsc)?;
    compiled_file(&modules, source)
}

fn compiled_file(modules: &HashMap<PathBuf, String>, source: &Path) -> Result<String, String> {
    lookup_js(&modules, source)
        .cloned()
        .ok_or_else(|| {
            format!(
                "{DEKA_VALIDATION_ERROR_MARKER}dsc did not emit {}",
                source.display()
            )
        })
}

fn collect_js(
    out_root: &Path,
    source_root: &Path,
    modules: &mut HashMap<PathBuf, String>,
) -> Result<(), String> {
    let source_root = fs::canonicalize(source_root).unwrap_or_else(|_| source_root.to_path_buf());
    collect_js_inner(out_root, out_root, &source_root, modules)
}

fn collect_js_inner(
    out_root: &Path,
    dir: &Path,
    source_root: &Path,
    modules: &mut HashMap<PathBuf, String>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("failed to read {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read {}: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_js_inner(out_root, &path, source_root, modules)?;
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

    #[test]
    fn lookup_js_matches_macos_private_var_alias() {
        let mut modules = HashMap::new();
        modules.insert(
            PathBuf::from("/var/folders/x/pkg/index.ds"),
            "export const n = 1".to_string(),
        );
        // Path is not on disk, so canonicalize leaves /private/var/... and
        // the /private prefix strip hits the map key.
        let hit = lookup_js(
            &modules,
            Path::new("/private/var/folders/x/pkg/index.ds"),
        );
        assert_eq!(hit.map(String::as_str), Some("export const n = 1"));
    }
}
