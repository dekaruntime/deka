//! Compile-root preparation for the closed `deka.*` catalog (RFD 21): scan
//! every DekaScript source under a project root, validate catalog calls, and
//! when a `safe` block needed lowering, build a staged mirror for dsc — the
//! user's tree is never modified. Staging verifies rewritten dependency
//! packages against the real `deka.lock` and re-pins only the mirror's lock
//! copy, because dsc verifies locked dependencies by hashing the sources it
//! reads.
//!
//! The scanner itself lives in [`crate::deka_catalog_scan`].

use std::fs;
use std::path::{Path, PathBuf};

use runtime_core::DEKA_VALIDATION_ERROR_MARKER;
use permissions::host_bridge;

use crate::deka_catalog_scan::{scan_source, ScanDiagnostic};

// ---- Compile-root preparation (scan + optional staging) ---------------------

/// Directories never mirrored into a compile stage and never scanned: VCS and
/// tooling state dsc cannot import, plus the cache trees themselves.
const SKIPPED_DIRS: &[&str] = &[".git", ".cache", "node_modules", "target"];

fn is_skipped_dir(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| SKIPPED_DIRS.contains(&name))
}

/// Manifest `name` at `root/deka.json`, when present and parseable.
fn manifest_name(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("deka.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("name")?.as_str().map(str::to_string)
}

/// Whether the source file at `path` belongs to an official `@deka/*` stdlib
/// package: a `@deka/*` dependency under `root/{ds_modules,php_modules}`, the
/// project root itself when its manifest names an official package, or a
/// linked package outside `root` whose nearest manifest is official.
pub fn is_stdlib_source(path: &Path, root: &Path) -> bool {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path = canonical.as_path();
    for dir in ["ds_modules", "php_modules"] {
        if let Ok(rel) = path.strip_prefix(root.join(dir)) {
            let mut components = rel.components();
            let Some(first) = components.next() else { return false };
            let first = first.as_os_str().to_string_lossy();
            let name = if first.starts_with('@') {
                let Some(second) = components.next() else { return false };
                format!("{first}/{}", second.as_os_str().to_string_lossy())
            } else {
                first.into_owned()
            };
            return host_bridge::is_official_package_name(&name);
        }
    }
    if path.starts_with(&root) {
        return manifest_name(&root)
            .map(|name| host_bridge::is_official_package_name(&name))
            .unwrap_or(false);
    }
    path.ancestors()
        .find(|ancestor| ancestor.join("deka.json").is_file())
        .and_then(|ancestor| manifest_name(ancestor))
        .map(|name| host_bridge::is_official_package_name(&name))
        .unwrap_or(false)
}

/// A staged compile root. Deletes the mirror on drop.
#[derive(Debug)]
pub struct StageGuard {
    dir: PathBuf,
    /// Original root this mirror reflects.
    source_root: PathBuf,
}

impl StageGuard {
    pub fn root(&self) -> &Path {
        &self.dir
    }

    /// Map an absolute source-root path to its staged equivalent. Input paths
    /// are canonicalized first: callers may hold the macOS `/var` spelling
    /// while the guard's roots are canonical `/private/var` paths.
    pub fn map_path(&self, path: &Path) -> PathBuf {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let rel = path.strip_prefix(&self.source_root).unwrap_or(&path);
        self.dir.join(rel)
    }

    /// Map an absolute staged path back to its source-root equivalent.
    pub fn unmap_path(&self, path: &Path) -> PathBuf {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let rel = path.strip_prefix(&self.dir).unwrap_or(&path);
        self.source_root.join(rel)
    }

    /// Rewrite staged paths in a dsc diagnostic back to source paths so
    /// diagnostics name the files the user actually owns.
    pub fn remap_diagnostic(&self, text: &str) -> String {
        text.replace(
            self.dir.to_string_lossy().as_ref(),
            self.source_root.to_string_lossy().as_ref(),
        )
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[derive(Debug)]
pub enum PreparedRoot {
    /// No `safe` blocks in the tree; compile against the real root.
    InPlace,
    /// At least one `safe` block was lowered; compile against the mirror.
    Staged(StageGuard),
}

/// Scan every DekaScript source under `root` for catalog calls, validating
/// against the closed catalog. When any `safe` block needed lowering, build a
/// staged mirror for dsc; otherwise report [`PreparedRoot::InPlace`].
pub fn prepare_compile_root(root: &Path) -> Result<PreparedRoot, String> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    // Collect candidate sources first (skip tooling dirs).
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !is_skipped_dir(&entry.file_name()) {
                    stack.push(path);
                }
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("ds" | "dsx")
            ) {
                sources.push(path);
            }
        }
    }
    sources.sort();

    let mut diagnostics: Vec<ScanDiagnostic> = Vec::new();
    let mut rewritten: Vec<(PathBuf, String)> = Vec::new();
    for path in sources {
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let stdlib = is_stdlib_source(&path, &root);
        let result = scan_source(&source, stdlib);
        // Offsets to 1-based line:col against the original source.
        let line_starts: Vec<usize> = std::iter::once(0usize)
            .chain(source.bytes().enumerate().filter_map(|(i, b)| (b == b'\n').then_some(i + 1)))
            .collect();
        let locate = |offset: usize| {
            let line = line_starts.partition_point(|&start| start <= offset);
            let column = offset - line_starts[line - 1] + 1;
            (line, column)
        };
        for (offset, message) in result.diagnostics {
            let (line, column) = locate(offset);
            diagnostics.push(ScanDiagnostic { path: path.clone(), line, column, message });
        }
        if let Some(text) = result.rewritten {
            rewritten.push((path, text));
        }
    }

    if !diagnostics.is_empty() {
        let detail = diagnostics
            .iter()
            .map(ScanDiagnostic::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("{DEKA_VALIDATION_ERROR_MARKER}{detail}"));
    }

    if rewritten.is_empty() {
        return Ok(PreparedRoot::InPlace);
    }

    // Stage: mirror the tree with hardlinks; lowered sources become real
    // copies inside the mirror only. The user's tree is never modified.
    // The name embeds a nanosecond stamp so concurrent compiles of the same
    // root never share a mirror.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stage = root.join(".cache").join(format!("dsc-stage-{}-{nonce}", std::process::id()));
    let _ = fs::remove_dir_all(&stage);
    mirror_tree(&root, &stage, &rewritten.iter().cloned().collect())?;
    // dsc verifies every locked dependency by hashing the sources it reads.
    // Inside the mirror those are the lowered sources, which cannot match the
    // lock's published-content hashes — so verify the real tree against the
    // real lock here (failing closed on any mismatch), then rewrite the
    // mirror's lock copy to describe the lowered mirror truthfully. The
    // user's deka.lock is never modified: the mirror holds a real copy.
    if let Err(err) = reconcile_staged_lock(&root, &stage, &rewritten) {
        let _ = fs::remove_dir_all(&stage);
        return Err(err);
    }
    Ok(PreparedRoot::Staged(StageGuard { dir: stage, source_root: root }))
}

/// Package directories under `ds_modules`/`php_modules` that contain at least
/// one rewritten source, as `(modules_dir, package_name)`.
fn rewritten_packages(
    root: &Path,
    rewritten: &[(PathBuf, String)],
) -> Vec<(String, String)> {
    let mut packages = Vec::new();
    for (path, _) in rewritten {
        for modules_dir in ["ds_modules", "php_modules"] {
            let Ok(rel) = path.strip_prefix(root.join(modules_dir)) else {
                continue;
            };
            let mut components = rel.components();
            let Some(first) = components.next() else { break };
            let first = first.as_os_str().to_string_lossy();
            let name = if first.starts_with('@') {
                let Some(second) = components.next() else { break };
                format!("{first}/{}", second.as_os_str().to_string_lossy())
            } else {
                first.into_owned()
            };
            let entry = (modules_dir.to_string(), name);
            if !packages.contains(&entry) {
                packages.push(entry);
            }
            break;
        }
    }
    packages
}

/// Verify rewritten dependency packages against the real lock and patch the
/// staged mirror's lock copy so dsc's own integrity check sees the lowered
/// content it actually compiles.
fn reconcile_staged_lock(
    root: &Path,
    stage: &Path,
    rewritten: &[(PathBuf, String)],
) -> Result<(), String> {
    let packages = rewritten_packages(root, rewritten);
    if packages.is_empty() {
        return Ok(());
    }
    let real_lock_path = root.join("deka.lock");
    let lock_text = fs::read_to_string(&real_lock_path).map_err(|err| {
        format!("{DEKA_VALIDATION_ERROR_MARKER}failed to read {}: {err}", real_lock_path.display())
    })?;
    let mut lock: serde_json::Value =
        serde_json::from_str(&lock_text).map_err(|err| {
            format!("{DEKA_VALIDATION_ERROR_MARKER}failed to parse {}: {err}", real_lock_path.display())
        })?;

    for (modules_dir, name) in &packages {
        let real_pkg = root.join(modules_dir).join(name);
        // The lock's fsGraph pins the published sources. Verify it before
        // anything compiles the lowered form; a mismatch fails closed with
        // the same remedy dsc's own check prints. Like dsc (compile_helper's
        // package_lock_error), only fsGraph is verified — legacy moduleGraph
        // entries self-heal and are never compared.
        let real = deka_host::integrity::compute_package_integrity(&real_pkg)
            .map_err(|err| format!("{DEKA_VALIDATION_ERROR_MARKER}{err}"))?;
        let mismatched = lock
            .get("packages")
            .and_then(|packages| packages.get(name))
            .and_then(|entry| entry.get(2))
            .map(|hashes| {
                hashes.pointer("/fsGraph/hash").and_then(|v| v.as_str()) != Some(real.fs_graph.as_str())
            })
            .unwrap_or(true);
        if mismatched {
            return Err(format!(
                "{DEKA_VALIDATION_ERROR_MARKER}❌ Integrity Mismatch\n\n    module '{name}' in {modules_dir}/ does not match deka.lock\n\n    = help: run `deka install` or `deka add {name}`. dsc does not fetch packages or write deka.lock."
            ));
        }

        // Lowering is semantics-preserving, so the mirror's fsGraph describes
        // the same package after a deterministic transform. Patching only
        // the mirror's fsGraph keeps dsc's check meaningful: it still
        // compares what it compiles against the staged tree, and anything
        // that rewrites a package without passing the real-lock verification
        // above cannot reach this point.
        let staged = deka_host::integrity::compute_package_integrity(&stage.join(modules_dir).join(name))
            .map_err(|err| format!("{DEKA_VALIDATION_ERROR_MARKER}{err}"))?;
        let entry = lock
            .pointer_mut(&format!("/packages/{}/2", name.replace('/', "~1")))
            .expect("lock entry verified above");
        entry["fsGraph"]["hash"] = serde_json::Value::String(staged.fs_graph);
    }

    // The mirror hardlinked the real lock; replace (never edit through the
    // link) so the user's lock file is untouched.
    let staged_lock = stage.join("deka.lock");
    let _ = fs::remove_file(&staged_lock);
    fs::write(&staged_lock, serde_json::to_string_pretty(&lock).map_err(|err| err.to_string())?)
        .map_err(|err| format!("failed to write {}: {err}", staged_lock.display()))?;
    Ok(())
}

/// Hardlink-copy `from` into `to`, replacing rewritten sources with their
/// lowered text. Hardlink failure (cross-device) falls back to a byte copy.
fn mirror_tree(
    from: &Path,
    to: &Path,
    rewritten: &std::collections::HashMap<PathBuf, String>,
) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|err| format!("failed to create {}: {err}", to.display()))?;
    let entries = fs::read_dir(from)
        .map_err(|err| format!("failed to read {}: {err}", from.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            if !is_skipped_dir(&entry.file_name()) {
                mirror_tree(&path, &target, rewritten)?;
            }
            continue;
        }
        if let Some(text) = rewritten.get(&path) {
            fs::write(&target, text)
                .map_err(|err| format!("failed to write {}: {err}", target.display()))?;
            continue;
        }
        match fs::hard_link(&path, &target) {
            Ok(()) => {}
            Err(_) => {
                fs::copy(&path, &target)
                    .map_err(|err| format!("failed to copy {}: {err}", path.display()))?;
            }
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_guard_maps_paths_both_ways() {
        let tmp = std::env::temp_dir().join(format!("deka-scan-{}", std::process::id()));
        let root = tmp.join("root");
        let stage = tmp.join("stage");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("main.ds"), "export const n = 1\n").unwrap();
        let guard = StageGuard {
            dir: stage.clone(),
            source_root: root.canonicalize().unwrap_or(root.clone()),
        };
        assert_eq!(guard.map_path(&root.join("main.ds")), stage.join("main.ds"));
        // unmap canonicalizes its input, so the result carries the canonical
        // (/private/var) root spelling on macOS.
        let expected = root.canonicalize().unwrap_or(root.clone()).join("main.ds");
        assert_eq!(guard.unmap_path(&stage.join("main.ds")), expected);
        let diag = guard.remap_diagnostic(&format!("{}:1:1: bad", stage.display()));
        assert!(diag.contains(&root.display().to_string()), "{diag}");
        drop(guard);
        let _ = fs::remove_dir_all(&tmp);
    }
    #[test]
    fn prepare_compile_root_lowers_safe_via_stage_and_cleans_up() {
        let tmp = std::env::temp_dir().join(format!("deka-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("ds_modules").join("@deka").join("demo");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(tmp.join("deka.json"), r#"{"name":"app"}"#).unwrap();
        fs::write(pkg.join("deka.json"), r#"{"name":"@deka/demo","version":"1.0.0"}"#).unwrap();
        fs::write(pkg.join("index.ds"), "export const n = safe { deka.time.now() }\n").unwrap();
        fs::write(tmp.join("main.ds"), "import { n } from \"demo\"\nexport const total = n\n").unwrap();
        // dsc verifies locked dependencies against deka.lock; staging rewrites
        // the package's sources, so the mirror's lock copy is re-pinned to the
        // lowered content after the real tree verified against the real lock.
        let integrity =
            deka_host::integrity::compute_package_integrity(&pkg).expect("package integrity");
        let lock = serde_json::json!({
            "lockfileVersion": 1,
            "packages": {
                "@deka/demo": [
                    "@deka/demo@1.0.0",
                    "linkhash:@deka/demo",
                    {
                        "repo": "https://github.com/dekaruntime/deka.git",
                        "gitRef": "v1.0.0",
                        "source": "deka.gg",
                        "dependencies": [],
                        "moduleGraph": { "algo": "sha256", "hash": integrity.module_graph },
                        "fsGraph": { "algo": "sha256", "hash": integrity.fs_graph }
                    },
                    ""
                ]
            }
        });
        let real_lock = serde_json::to_string_pretty(&lock).unwrap();
        fs::write(tmp.join("deka.lock"), &real_lock).unwrap();

        let prepared = prepare_compile_root(&tmp).expect("prepare");
        let PreparedRoot::Staged(guard) = prepared else {
            panic!("safe block must stage the compile");
        };
        // The user tree is untouched; the mirror carries the lowered source.
        let original = fs::read_to_string(pkg.join("index.ds")).unwrap();
        assert!(original.contains("safe {"), "user tree never modified");
        let lowered = fs::read_to_string(guard.map_path(&pkg.join("index.ds"))).unwrap();
        assert_eq!(lowered, "export const n = deka.time.now()\n");
        assert!(guard.root().join("main.ds").is_file(), "entry mirrored");
        // The user's lock is byte-identical after staging — the mirror holds
        // a real copy, never an edit through the hardlink.
        assert_eq!(fs::read_to_string(tmp.join("deka.lock")).unwrap(), real_lock);
        // The mirror's lock copy is re-pinned to the lowered content, which
        // is exactly what dsc will hash when it verifies the package.
        let staged_lock: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(guard.root().join("deka.lock")).unwrap(),
        )
        .unwrap();
        let staged_hash = staged_lock["packages"]["@deka/demo"][2]["fsGraph"]["hash"]
            .as_str()
            .unwrap();
        let staged_pkg = guard.map_path(&pkg);
        let staged_integrity =
            deka_host::integrity::compute_package_integrity(&staged_pkg).unwrap();
        assert_eq!(staged_hash, staged_integrity.fs_graph);
        assert_ne!(staged_hash, integrity.fs_graph, "lowered content hashes differently");
        let stage_dir = guard.root().to_path_buf();
        drop(guard);
        assert!(!stage_dir.exists(), "stage mirror removed on drop");
        let _ = fs::remove_dir_all(&tmp);
    }
    #[test]
    fn prepare_compile_root_rejects_rewritten_package_that_does_not_match_lock() {
        let tmp = std::env::temp_dir().join(format!("deka-lockneg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("ds_modules").join("@deka").join("demo");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(tmp.join("deka.json"), r#"{"name":"app"}"#).unwrap();
        fs::write(pkg.join("deka.json"), r#"{"name":"@deka/demo","version":"1.0.0"}"#).unwrap();
        fs::write(pkg.join("index.ds"), "export const n = safe { deka.time.now() }\n").unwrap();
        fs::write(tmp.join("main.ds"), "import { n } from \"demo\"\nexport const total = n\n").unwrap();
        // Pin the lock against different (published) content, then let the
        // on-disk package drift: staging must fail closed, not re-pin a
        // tampered package.
        let integrity =
            deka_host::integrity::compute_package_integrity(&pkg).expect("package integrity");
        fs::write(
            tmp.join("deka.lock"),
            serde_json::to_string(&serde_json::json!({
                "lockfileVersion": 1,
                "packages": {
                    "@deka/demo": [
                        "@deka/demo@1.0.0",
                        "linkhash:@deka/demo",
                        {
                            "moduleGraph": { "algo": "sha256", "hash": integrity.module_graph },
                            "fsGraph": { "algo": "sha256", "hash": integrity.fs_graph }
                        },
                        ""
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(pkg.join("index.ds"), "export const n = safe { deka.time.now() }\nexport const drift = 1\n").unwrap();

        let err = prepare_compile_root(&tmp).expect_err("tampered package must not stage");
        assert!(
            err.contains("Integrity Mismatch") && err.contains("@deka/demo"),
            "{err}"
        );
        // A failed staging must not leave a mirror behind.
        let leftover = fs::read_dir(tmp.join(".cache"))
            .map(|entries| entries.flatten().count())
            .unwrap_or(0);
        assert_eq!(leftover, 0, "failed staging must clean up its mirror: {err}");
        let _ = fs::remove_dir_all(&tmp);
    }
    #[test]
    fn prepare_compile_root_without_safe_compiles_in_place() {
        let tmp = std::env::temp_dir().join(format!("deka-inplace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("deka.json"), r#"{"name":"app"}"#).unwrap();
        fs::write(tmp.join("main.ds"), "export const n = 1\n").unwrap();
        let prepared = prepare_compile_root(&tmp).expect("prepare");
        assert!(matches!(prepared, PreparedRoot::InPlace));
        let _ = fs::remove_dir_all(&tmp);
    }
    #[test]
    fn prepare_compile_root_reports_source_diagnostics_with_locations() {
        let tmp = std::env::temp_dir().join(format!("deka-diag-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("ds_modules").join("@deka").join("demo");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("deka.json"), r#"{"name":"@deka/demo"}"#).unwrap();
        fs::write(pkg.join("index.ds"), "export const n = safe { deka.bytes.bogus(b) }\n").unwrap();
        let err = prepare_compile_root(&tmp).expect_err("diagnostic");
        assert!(err.contains(&pkg.join("index.ds").display().to_string()), "{err}");
        assert!(err.contains(":1:25:"), "{err}");
        assert!(err.contains("unknown `deka.bytes` helper `bogus`"), "{err}");
        let _ = fs::remove_dir_all(&tmp);
    }
    #[test]
    fn prepare_compile_root_flags_user_package_catalog_use() {
        let tmp = std::env::temp_dir().join(format!("deka-user-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("deka.json"), r#"{"name":"app"}"#).unwrap();
        fs::write(tmp.join("main.ds"), "const n = unsafe { deka.json.parse(s) }\nexport const x = n\n").unwrap();
        let err = prepare_compile_root(&tmp).expect_err("diagnostic");
        assert!(err.contains("stdlib-only"), "{err}");
        let _ = fs::remove_dir_all(&tmp);
    }
}
