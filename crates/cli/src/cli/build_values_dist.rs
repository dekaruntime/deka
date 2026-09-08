//! Self-contained build-value modules in published dist (deka#738 F7).
//!
//! `deka build` emits app JS whose build slots import `deka:dev/<id>` — a
//! dev-scheme specifier the serve-time loader resolves only from the project
//! compiler cache (`.cache/dekascript/build-values/`). A deployment carrying
//! only `dist/` therefore 500s on every build-backed route. This module makes
//! the published tree self-contained: copy each materialized slot module into
//! `dist/app/.build-values/` and rewrite the specifier in every emitted JS
//! file to a relative path inside dist. `deka dev` keeps resolving
//! `deka:dev/<id>` from the cache unchanged.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Where the materialized build-value modules live inside the app tree.
pub const BUILD_VALUES_DIR: &str = ".build-values";

/// Ship this build's materialized build-value modules inside the staged dist
/// tree and rewrite every `deka:dev/<id>` specifier in emitted JS to a
/// relative path into them. No-op for builds without build slots. Must run
/// before `build_publish::finalize` so the manifest hashes the shipped
/// modules. Fails the build if any dev-scheme specifier survives the rewrite
/// — the staged tree is then not deployable and must not publish.
pub fn publish_build_values(
    project_root: &Path,
    dist_root: &Path,
    dist_app: &Path,
    planned: &[runtime_core::framework::PlannedSource],
) -> Result<(), String> {
    let ids: BTreeSet<String> = planned
        .iter()
        .flat_map(|source| source.plan.slots.iter().map(|slot| slot.id.clone()))
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    copy_build_value_modules(
        &runtime_core::framework::compiler_cache_dir(project_root).join("build-values"),
        dist_app,
        &ids,
    )?;
    rewrite_build_value_specifiers(dist_root, dist_app, &ids)
}

/// Copy the materialized build-value modules for `ids` from the project
/// cache into `<dist_app>/.build-values/`. Only ids referenced by this
/// build are copied; the staged publish replaces dist wholesale, so no
/// stale-module sweep is needed here.
pub fn copy_build_value_modules(
    cache_values_dir: &Path,
    dist_app: &Path,
    ids: &BTreeSet<String>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let target_dir = dist_app.join(BUILD_VALUES_DIR);
    fs::create_dir_all(&target_dir)
        .map_err(|err| format!("failed to create {}: {err}", target_dir.display()))?;
    for id in ids {
        let source = cache_values_dir.join(format!("{id}.js"));
        if !source.is_file() {
            return Err(format!(
                "build value `{}` was not materialized at {}",
                id,
                source.display()
            ));
        }
        fs::copy(&source, target_dir.join(format!("{id}.js"))).map_err(|err| {
            format!(
                "failed to copy {} -> {}: {err}",
                source.display(),
                target_dir.join(format!("{id}.js")).display()
            )
        })?;
    }
    Ok(())
}

/// Rewrite every `"deka:dev/<id>"` literal (quotes included) in all `.js`
/// files under `dist_root` to the relative path from the importing file to
/// `<dist_app>/.build-values/<id>.js` (forward slashes). Only ids in `ids`
/// are rewritten. Fails the build loudly if any `deka:dev/` specifier
/// survives: that means a specifier appeared this build never copied a
/// backing module for.
pub fn rewrite_build_value_specifiers(
    dist_root: &Path,
    dist_app: &Path,
    ids: &BTreeSet<String>,
) -> Result<(), String> {
    if ids.is_empty() {
        assert_no_dev_specifiers(dist_root)?;
        return Ok(());
    }
    let modules_dir = dist_app.join(BUILD_VALUES_DIR);
    let mut js_files = Vec::new();
    collect_js_files(dist_root, &mut js_files)?;
    for file in &js_files {
        let mut source = fs::read_to_string(file)
            .map_err(|err| format!("failed to read {}: {err}", file.display()))?;
        if !source.contains("deka:dev/") {
            continue;
        }
        let importer_dir = file.parent().unwrap_or(dist_root);
        for id in ids {
            let needle = format!("\"deka:dev/{id}\"");
            if !source.contains(&needle) {
                continue;
            }
            let target = modules_dir.join(format!("{id}.js"));
            let rewritten = format!("\"{}\"", relative_path(importer_dir, &target));
            source = source.replace(&needle, &rewritten);
        }
        fs::write(file, source)
            .map_err(|err| format!("failed to write {}: {err}", file.display()))?;
    }
    assert_no_dev_specifiers(dist_root)
}

/// A dev-scheme specifier left anywhere in the staged dist tree means the
/// output is not deployable — fail the build instead of publishing it.
fn assert_no_dev_specifiers(dist_root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect_files(dist_root, &mut files)?;
    for file in files {
        let bytes =
            fs::read(&file).map_err(|err| format!("failed to read {}: {err}", file.display()))?;
        if bytes.windows(b"deka:dev/".len()).any(|w| w == b"deka:dev/") {
            return Err(format!(
                "build output is not self-contained: `{}` still references a deka:dev/ specifier",
                file.display()
            ));
        }
    }
    Ok(())
}

fn collect_js_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    collect_files(dir, out)?;
    out.retain(|path| {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("js"))
    });
    Ok(())
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Forward-slash relative path from `from_dir` to `to_file`, `./`-prefixed.
/// Both must sit under the same tree (the staged dist root and its files do).
pub fn relative_path(from_dir: &Path, to_file: &Path) -> String {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to_file.components().collect();
    let mut shared = 0;
    while shared < from.len() && shared < to.len() && from[shared] == to[shared] {
        shared += 1;
    }
    let mut parts: Vec<String> = (shared..from.len()).map(|_| "..".to_string()).collect();
    parts.extend(to[shared..].iter().map(|component| {
        component.as_os_str().to_string_lossy().into_owned()
    }));
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, body).expect("write");
    }

    #[test]
    fn relative_path_computes_forward_slash_parents() {
        let root = Path::new("/project/dist");
        let dist_app = root.join("app");
        let importer = root.join("app").join("posts").join("[slug]");
        let target = dist_app.join(BUILD_VALUES_DIR).join("abc.js");
        assert_eq!(
            relative_path(&importer, &target),
            "../../.build-values/abc.js"
        );
        assert_eq!(
            relative_path(&root.join("app"), &target),
            ".build-values/abc.js"
        );
    }

    #[test]
    fn rewrite_swaps_known_ids_and_keeps_dist_self_contained() {
        let project = tempfile::tempdir().unwrap();
        let dist = project.path().join("dist");
        let dist_app = dist.join("app");
        write(
            &dist_app.join("posts").join("[slug]"),
            "page.js",
            "import { hydrate as b } from \"deka:dev/abc123\";\nconst page = 1;\n",
        );
        let ids: BTreeSet<String> = ["abc123".to_string()].into_iter().collect();
        rewrite_build_value_specifiers(&dist, &dist_app, &ids).unwrap();
        let rewritten = fs::read_to_string(
            dist_app.join("posts").join("[slug]").join("page.js"),
        )
        .unwrap();
        assert!(
            rewritten.contains("\"../../.build-values/abc123.js\""),
            "{rewritten}"
        );
        assert!(!rewritten.contains("deka:dev/"), "{rewritten}");
    }

    #[test]
    fn rewrite_fails_loudly_on_unmaterialized_specifier() {
        let project = tempfile::tempdir().unwrap();
        let dist = project.path().join("dist");
        let dist_app = dist.join("app");
        write(
            &dist_app,
            "page.js",
            "import { hydrate as b } from \"deka:dev/unknown\";\n",
        );
        let ids: BTreeSet<String> = ["abc123".to_string()].into_iter().collect();
        let err = rewrite_build_value_specifiers(&dist, &dist_app, &ids)
            .expect_err("an unknown deka:dev specifier must fail the build");
        assert!(err.contains("not self-contained"), "{err}");
    }

    #[test]
    fn copy_pulls_only_referenced_ids() {
        let project = tempfile::tempdir().unwrap();
        let cache = project.path().join("cache").join("build-values");
        let dist_app = project.path().join("dist").join("app");
        write(&cache, "abc.js", "export const value = 1;\n");
        write(&cache, "stale.js", "export const value = 2;\n");
        let ids: BTreeSet<String> = ["abc".to_string()].into_iter().collect();
        copy_build_value_modules(&cache, &dist_app, &ids).unwrap();
        assert!(dist_app.join(BUILD_VALUES_DIR).join("abc.js").is_file());
        assert!(!dist_app.join(BUILD_VALUES_DIR).join("stale.js").exists());
    }
}
