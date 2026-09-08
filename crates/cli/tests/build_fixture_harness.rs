//! deka#720: deterministic-artifact contract harness for `deka build`.
//! Shared by `build_artifacts.rs` via `#[path]` (precedent:
//! `crates/runtime_core/src/framework/build_manifest_tests.rs`).
//!
//! Each fixture lives in `tests/fixtures/build/<name>/`:
//!
//! ```text
//! project/             the deka project source (copied to a tempdir)
//! expected/
//!   stderr.txt         expected stderr after path normalization, including
//!                      the route table and diagnostics
//!   stderr.v1.txt      optional: expected stderr when dsc emits plan v1
//!                      (dsc 0.6.x); used instead of stderr.txt on v1
//!   fail               optional marker: build must fail; dist/ must not exist
//!   v1-fail            optional marker: under plan v1 the build must fail
//!                      (used with stderr.v1.txt for prerender = false fixtures)
//!   tree.txt           canonical sorted dist/ relative paths (success only)
//!   files/             mirror of dist/ holding expected BYTES for every file
//!                      in tree.txt (success only)
//! ```
//!
//! A successful fixture builds THREE times: a same-root rerun (tree, full
//! normalized stderr, and the build manifest must be byte-identical across
//! runs) and a build from a SECOND temporary root (the RAW dist bytes must
//! be byte-identical across roots — dsc slot ids are project-relative since
//! dsc PR #62, so no byte normalization exists anywhere in this harness;
//! deka#728 / codex findings 5 and 6). The manifest's artifact digests are
//! recomputed against the published bytes, not just its path list.
//! `DEKA_BLESS=1` regenerates the expected files from actual output — the
//! only way expected bytes may change.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("build")
}

fn real_dsc() -> PathBuf {
    if let Ok(path) = std::env::var("DEKA_DSC") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let output = Command::new("which")
        .arg("dsc")
        .output()
        .expect("run which dsc");
    assert!(
        output.status.success(),
        "build fixtures require dsc (set DEKA_DSC or put dsc on PATH)"
    );
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

/// The build plan version the installed dsc emits; decides whether
/// `prerender = false` fixtures run their v2 (success) or v1 (fail-closed)
/// variant. Probed once per test process.
fn dsc_plan_version() -> u32 {
    static VERSION: OnceLock<u32> = OnceLock::new();
    *VERSION.get_or_init(|| {
        let dir = tempfile::tempdir().expect("create probe dir");
        let file = dir.path().join("probe.ds");
        fs::write(&file, "export const prerender = false\n").expect("write probe");
        let output = Command::new(real_dsc())
            .arg("plan")
            .arg(&file)
            .current_dir(dir.path())
            .output()
            .expect("run dsc plan probe");
        assert!(
            output.status.success(),
            "dsc plan probe failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let plan: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("parse probe plan");
        plan.get("version")
            .and_then(|v| v.as_u64())
            .expect("probe plan has a version") as u32
    })
}

struct BuildRun {
    success: bool,
    stdout: String,
    stderr: String,
}

fn run_build(project: &Path) -> BuildRun {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(project)
        .output()
        .expect("run deka build");
    BuildRun {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Replace the tempdir path (canonical and pre-canonical spellings) so
/// expected stderr is machine-independent.
fn normalize_stderr(project: &Path, project_canonical: &Path, stderr: &str) -> String {
    let mut out = stderr.replace(
        project_canonical.to_str().expect("utf8 path"),
        "<project>",
    );
    if project != project_canonical {
        out = out.replace(project.to_str().expect("utf8 path"), "<project>");
    }
    out
}

/// Walk `dir` into a forward-slash relative-path -> bytes map.
/// RAW bytes: with project-relative dsc slot ids (dsc PR #62), build output
/// is byte-identical across project roots, so no normalization belongs here
/// (deka#728, codex finding 5).
fn tree_snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).expect("read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .expect("rel")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, fs::read(&path).expect("read"));
            }
        }
    }
    out
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("mkdir");
    for entry in fs::read_dir(src).expect("read_dir").flatten() {
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).expect("copy");
        }
    }
}

/// Route-table rows (`glyph path`, detail column dropped) in printed order.
/// Ordering matters: the manifest derives the table, so row order is part of
/// the determinism contract.
fn route_table_rows(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let is_row = trimmed.starts_with("○ ")
                || trimmed.starts_with("● ")
                || trimmed.starts_with('ƒ')
                || trimmed.starts_with('λ');
            if !is_row {
                return None;
            }
            let mut tokens = trimmed.split_whitespace();
            let glyph = tokens.next()?;
            let path = tokens.next()?;
            Some(format!("{glyph} {path}"))
        })
        .collect()
}

/// A small line-based unified diff (3 lines of context) for readable
/// mismatch reports. Build artifacts are at most a few thousand lines, so
/// the quadratic LCS table is fine; larger inputs fall back to a plain dump.
fn unified_diff(expected: &str, actual: &str) -> String {
    let a: Vec<&str> = expected.lines().collect();
    let b: Vec<&str> = actual.lines().collect();
    let mut out = String::from("--- expected\n+++ actual\n");
    if a.len().saturating_mul(b.len()) > 4_000_000 {
        let _ = writeln!(
            out,
            "@@ large file: expected {} lines, actual {} lines @@",
            a.len(),
            b.len()
        );
        let _ = writeln!(out, "expected first line: {:?}", a.first());
        let _ = writeln!(out, "actual   first line: {:?}", b.first());
        return out;
    }
    // LCS length table (full, for backtracking).
    let mut table: Vec<Vec<usize>> = Vec::with_capacity(a.len() + 1);
    table.push(vec![0; b.len() + 1]);
    for (i, line_a) in a.iter().enumerate() {
        let prev = &table[i];
        let mut curr = vec![0; b.len() + 1];
        for (j, line_b) in b.iter().enumerate() {
            curr[j + 1] = if line_a == line_b {
                prev[j] + 1
            } else {
                prev[j + 1].max(curr[j])
            };
        }
        table.push(curr);
    }
    // Backtrack into an edit script: (' ', kept) ('-', expected only)
    // ('+', actual only).
    let mut ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (a.len(), b.len());
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            ops.push((' ', a[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            ops.push(('+', b[j - 1]));
            j -= 1;
        } else {
            ops.push(('-', a[i - 1]));
            i -= 1;
        }
    }
    ops.reverse();
    // Group changes into hunks with 3 lines of context; merge hunks whose
    // context windows overlap.
    let context = 3usize;
    let changes: Vec<usize> = (0..ops.len()).filter(|&k| ops[k].0 != ' ').collect();
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for &c in &changes {
        match hunks.last_mut() {
            Some((_, last)) if c <= *last + 2 * context + 1 => *last = c,
            _ => hunks.push((c, c)),
        }
    }
    for (first, last) in hunks {
        let start = first.saturating_sub(context);
        let end = (last + context + 1).min(ops.len());
        let a_start = ops[..start].iter().filter(|op| op.0 != '+').count();
        let b_start = ops[..start].iter().filter(|op| op.0 != '-').count();
        let _ = writeln!(out, "@@ -{a_start} +{b_start} @@");
        for (op, line) in &ops[start..end] {
            let _ = writeln!(out, "{op}{line}");
        }
    }
    out
}

fn manifest_path(project: &Path) -> PathBuf {
    project
        .join(".cache")
        .join("dekascript")
        .join("build-manifest.json")
}

/// The manifest is the host-owned record the table derives from: its routes
/// must match the printed table (glyph + path, in order) and its artifact
/// list must cover exactly the published dist tree.
fn manifest_report(project: &Path, stderr: &str, tree: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let raw = fs::read_to_string(manifest_path(project))
        .map_err(|err| format!("build-manifest.json unreadable: {err}"))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&raw).map_err(|err| format!("build-manifest.json unparseable: {err}"))?;
    let routes = manifest["routes"]
        .as_array()
        .ok_or("build-manifest.json has no routes array")?;
    let mut expected_rows = Vec::new();
    for route in routes {
        let glyph = match route["mode"].as_str() {
            Some("static") => "○",
            Some("static_params") => "●",
            Some("request_time") => "ƒ",
            Some("api") => "λ",
            other => return Err(format!("route has unknown mode {other:?}")),
        };
        let path = route["instance"]
            .as_str()
            .or_else(|| route["template"].as_str())
            .ok_or("route has neither instance nor template")?;
        expected_rows.push(format!("{glyph} {path}"));
    }
    let printed = route_table_rows(stderr);
    if printed != expected_rows {
        return Err(format!(
            "route table disagrees with build-manifest.json routes:\n{}",
            unified_diff(&expected_rows.join("\n"), &printed.join("\n"))
        ));
    }
    let artifacts = manifest["artifacts"]
        .as_array()
        .ok_or("build-manifest.json has no artifacts array")?;
    let artifact_paths: std::collections::BTreeSet<String> = artifacts
        .iter()
        .filter_map(|artifact| artifact["path"].as_str().map(str::to_string))
        .collect();
    let tree_paths: std::collections::BTreeSet<String> = tree.keys().cloned().collect();
    if artifact_paths != tree_paths {
        return Err(format!(
            "manifest artifacts disagree with the dist tree:\n  only in manifest: {:?}\n  only in dist: {:?}",
            artifact_paths.difference(&tree_paths).collect::<Vec<_>>(),
            tree_paths.difference(&artifact_paths).collect::<Vec<_>>()
        ));
    }
    // Digests: every declared artifact must hash to the published bytes.
    // Path coverage alone would pass a manifest whose digests are all wrong
    // (deka#728, codex finding 6).
    for artifact in artifacts {
        let path = artifact["path"]
            .as_str()
            .ok_or("artifact has no path")?;
        let digest = artifact["digest"]
            .as_str()
            .ok_or_else(|| format!("artifact `{path}` has no digest"))?;
        let bytes = tree
            .get(path)
            .ok_or_else(|| format!("artifact `{path}` is not in the dist tree"))?;
        let actual = sha256_hex(bytes);
        if actual != digest {
            return Err(format!(
                "artifact `{path}` digest mismatch: manifest declares {digest}, published bytes hash to {actual}"
            ));
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

struct Fixture {
    name: String,
    root: PathBuf,
}

impl Fixture {
    fn load(name: &str) -> Self {
        let root = fixtures_dir().join(name);
        assert!(
            root.join("project").is_dir(),
            "fixture `{name}` has no project/ directory"
        );
        Fixture {
            name: name.to_string(),
            root,
        }
    }

    fn expected(&self) -> PathBuf {
        self.root.join("expected")
    }

    fn stderr_file(&self, plan_version: u32) -> String {
        // The v1 variant is selected when it already exists OR when the
        // fixture declares a v1-specific expectation (v1-fail); the latter
        // lets a first bless under v1 create stderr.v1.txt (deka#720:
        // request-time fails closed on v1 dsc, so its v1 stderr is a
        // failure diagnostic, not the v2 route table).
        let expected = self.expected();
        if plan_version < 2
            && (expected.join("stderr.v1.txt").is_file() || expected.join("v1-fail").is_file())
        {
            "stderr.v1.txt".to_string()
        } else {
            "stderr.txt".to_string()
        }
    }

    fn expects_failure(&self, plan_version: u32) -> bool {
        self.expected().join("fail").is_file()
            || (plan_version < 2 && self.expected().join("v1-fail").is_file())
    }
}

fn bless(
    fixture: &Fixture,
    stderr_name: &str,
    expects_failure: bool,
    run: &BuildRun,
    stderr: &str,
    project: &Path,
) {
    assert!(
        run.stdout.is_empty(),
        "cannot bless `{}`: stdout must be empty (stdio convention), got {:?}",
        fixture.name,
        run.stdout
    );
    assert_eq!(
        run.success, !expects_failure,
        "cannot bless `{}`: build {} but fixture expects {}",
        fixture.name,
        if run.success { "succeeded" } else { "failed" },
        if expects_failure { "failure" } else { "success" }
    );
    let expected_dir = fixture.expected();
    fs::create_dir_all(&expected_dir).expect("mkdir expected");
    fs::write(expected_dir.join(stderr_name), stderr).expect("write blessed stderr");
    if !expects_failure {
        let dist = project.join("dist");
        let tree = tree_snapshot(&dist);
        fs::write(
            expected_dir.join("tree.txt"),
            format!("{}\n", tree.keys().cloned().collect::<Vec<_>>().join("\n")),
        )
        .expect("write blessed tree.txt");
        let mirror = expected_dir.join("files");
        if mirror.exists() {
            fs::remove_dir_all(&mirror).expect("clear blessed files/");
        }
        for (rel, bytes) in &tree {
            let dest = mirror.join(rel);
            fs::create_dir_all(dest.parent().expect("parent")).expect("mkdir");
            fs::write(&dest, bytes).expect("write blessed file");
        }
    }
    eprintln!(
        "DEKA_BLESS: regenerated {} for fixture `{}`",
        stderr_name, fixture.name
    );
}

/// Run one build fixture end to end and assert the deterministic-artifact
/// contract. Panics with a path-labeled report on any mismatch.
pub fn check_fixture(name: &str) {
    let fixture = Fixture::load(name);
    let plan_version = dsc_plan_version();
    let stderr_name = fixture.stderr_file(plan_version);
    let expects_failure = fixture.expects_failure(plan_version);
    eprintln!(
        "fixture `{name}`: dsc plan v{plan_version}, expecting {}, stderr `{stderr_name}`",
        if expects_failure { "failure" } else { "success" }
    );

    let tmp = tempfile::tempdir().expect("create temp project dir");
    let project = tmp.path().join("project");
    copy_dir(&fixture.root.join("project"), &project);
    let project_canonical = fs::canonicalize(&project).expect("canonicalize project");

    let run = run_build(&project);
    let stderr = normalize_stderr(&project, &project_canonical, &run.stderr);

    if std::env::var("DEKA_BLESS").ok().as_deref() == Some("1") {
        bless(&fixture, &stderr_name, expects_failure, &run, &stderr, &project);
        return;
    }

    let mut problems: Vec<String> = Vec::new();
    if !run.stdout.is_empty() {
        problems.push(format!("[stdout] expected empty, got:\n{}", run.stdout));
    }
    if run.success != !expects_failure {
        problems.push(format!(
            "[exit] expected {}, build {}\nstderr:\n{}",
            if expects_failure { "failure" } else { "success" },
            if run.success { "succeeded" } else { "failed" },
            stderr
        ));
    }
    let expected_stderr = fs::read_to_string(fixture.expected().join(&stderr_name))
        .unwrap_or_else(|err| panic!("fixture `{name}`: cannot read expected/{stderr_name}: {err}"));
    if stderr != expected_stderr {
        problems.push(format!(
            "[stderr] mismatch vs expected/{stderr_name}\n{}",
            unified_diff(&expected_stderr, &stderr)
        ));
    }

    if !expects_failure {
        check_published_output(&fixture, &project, &stderr, &mut problems);
    } else if project.join("dist").exists() {
        problems.push("[tree] a failed build must not publish dist/".to_string());
    }

    if !problems.is_empty() {
        let mut report = format!("fixture `{name}` FAILED (dsc plan v{plan_version})\n\n");
        report.push_str(&problems.join("\n\n"));
        report.push_str("\n\nTo accept intentional output changes, rerun with DEKA_BLESS=1.");
        panic!("{report}");
    }
}

/// Tree, bytes, manifest, and repeated-build determinism checks for a
/// successful fixture. Appends human-readable problems to `problems`.
fn check_published_output(
    fixture: &Fixture,
    project: &Path,
    stderr: &str,
    problems: &mut Vec<String>,
) {
    let name = &fixture.name;
    let dist = project.join("dist");
    let tree = tree_snapshot(&dist);

    // 1. File tree: canonical sorted relative paths, missing/unexpected
    //    spelled out.
    let tree_txt = fs::read_to_string(fixture.expected().join("tree.txt"))
        .unwrap_or_else(|err| panic!("fixture `{name}`: cannot read expected/tree.txt: {err}"));
    let expected_paths: std::collections::BTreeSet<String> = tree_txt
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let actual_paths: std::collections::BTreeSet<String> = tree.keys().cloned().collect();
    if expected_paths != actual_paths {
        let mut msg = String::from("[tree] dist/ file set mismatch");
        let missing: Vec<String> = expected_paths.difference(&actual_paths).cloned().collect();
        let unexpected: Vec<String> = actual_paths.difference(&expected_paths).cloned().collect();
        if !missing.is_empty() {
            let _ = write!(msg, "\nmissing (expected but not built):\n  {}", missing.join("\n  "));
        }
        if !unexpected.is_empty() {
            let _ = write!(
                msg,
                "\nunexpected (built but not expected):\n  {}",
                unexpected.join("\n  ")
            );
        }
        problems.push(msg);
    }

    // 2. Exact bytes of every declared output, from the expected mirror.
    let mirror = fixture.expected().join("files");
    for rel in &actual_paths {
        let expected_bytes = match fs::read(mirror.join(rel)) {
            Ok(bytes) => bytes,
            Err(err) => {
                problems.push(format!("[bytes] expected mirror lacks `{rel}`: {err}"));
                continue;
            }
        };
        let actual_bytes = &tree[rel];
        if *actual_bytes != expected_bytes {
            let header = format!("[bytes] `{rel}` differs from expected mirror");
            let detail = match (
                std::str::from_utf8(&expected_bytes),
                std::str::from_utf8(actual_bytes),
            ) {
                (Ok(expected), Ok(actual)) => unified_diff(expected, actual),
                _ => format!(
                    "binary file differs: expected {} bytes, actual {} bytes",
                    expected_bytes.len(),
                    actual_bytes.len()
                ),
            };
            problems.push(format!("{header}\n{detail}"));
        }
    }

    // 3. The manifest must parse, agree with the printed table, and cover
    //    exactly the published tree.
    if let Err(err) = manifest_report(project, stderr, &tree) {
        problems.push(format!("[manifest] {err}"));
    }

    // 4. Same-root rerun: tree, route-table order (full stderr), and the
    //    manifest must all be byte-identical. The manifest embeds absolute
    //    paths, so byte-identity is only meaningful within one root.
    let canonical = fs::canonicalize(project).expect("canonicalize project");
    let rerun = run_build(project);
    if !rerun.success {
        problems.push(format!(
            "[rerun] second build failed:\n{}",
            normalize_stderr(project, &canonical, &rerun.stderr)
        ));
        return;
    }
    let rerun_stderr = normalize_stderr(project, &canonical, &rerun.stderr);
    if rerun_stderr != stderr {
        problems.push(format!(
            "[rerun] stderr (route table included) changed between builds\n{}",
            unified_diff(stderr, &rerun_stderr)
        ));
    }
    let rerun_tree = tree_snapshot(&dist);
    if rerun_tree != tree {
        let changed: Vec<_> = rerun_tree
            .iter()
            .filter(|(rel, bytes)| tree.get(*rel) != Some(*bytes))
            .map(|(rel, _)| rel.clone())
            .collect();
        problems.push(format!(
            "[rerun] dist/ changed between builds, differing files: {changed:?}"
        ));
    }
    let manifest_a = fs::read(manifest_path(project)).expect("read manifest (first build)");
    let manifest_b = fs::read(manifest_path(project)).expect("read manifest (rerun)");
    if manifest_a != manifest_b {
        problems.push("[rerun] build-manifest.json is not byte-identical across builds".to_string());
    }
    if project.join(".deka-dist-stage").exists() {
        problems.push("[rerun] staging dir leaked into the project".to_string());
    }

    // 5. Cross-root determinism (deka#728, codex finding 5): build the SAME
    //    project from a DIFFERENT temporary root and require RAW,
    //    byte-identical artifacts — no normalization anywhere. This holds
    //    because dsc slot ids are project-relative (dsc PR #62). If it ever
    //    fails, investigate which bytes embed the root before considering
    //    any normalization.
    let tmp_b = tempfile::tempdir().expect("create second temp project dir");
    let project_b = tmp_b.path().join("project");
    copy_dir(&fixture.root.join("project"), &project_b);
    let canonical_b = fs::canonicalize(&project_b).expect("canonicalize second project");
    let cross = run_build(&project_b);
    if !cross.success {
        problems.push(format!(
            "[cross-root] build failed in the second root:\n{}",
            normalize_stderr(&project_b, &canonical_b, &cross.stderr)
        ));
        return;
    }
    let cross_stderr = normalize_stderr(&project_b, &canonical_b, &cross.stderr);
    if cross_stderr != stderr {
        problems.push(format!(
            "[cross-root] stderr changed across roots\n{}",
            unified_diff(stderr, &cross_stderr)
        ));
    }
    let cross_tree = tree_snapshot(&project_b.join("dist"));
    if cross_tree != tree {
        let changed: Vec<_> = cross_tree
            .iter()
            .filter(|(rel, bytes)| tree.get(*rel) != Some(*bytes))
            .map(|(rel, _)| rel.clone())
            .collect();
        problems.push(format!(
            "[cross-root] dist/ bytes differ across roots, differing files: {changed:?}"
        ));
    }
}

#[cfg(test)]
mod digest_verification {
    use super::*;

    /// A manifest whose artifact digests do not match the published bytes
    /// must fail verification and name the mismatched path (deka#728, codex
    /// finding 6). Constructed directly — a real build, then one digest
    /// tampered on disk — not a mock.
    #[test]
    fn tampered_manifest_digest_fails() {
        let source = fixtures_dir().join("static-site").join("project");
        let tmp = tempfile::tempdir().expect("create temp project dir");
        let project = tmp.path().join("project");
        copy_dir(&source, &project);
        let run = run_build(&project);
        assert!(run.success, "fixture build must succeed: {}", run.stderr);
        let tree = tree_snapshot(&project.join("dist"));
        manifest_report(&project, &run.stderr, &tree).expect("untampered manifest verifies");

        // Flip every digest character; the result cannot equal the real
        // digest, so verification must fail on exactly this artifact.
        let path = manifest_path(&project);
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read manifest"))
                .expect("parse manifest");
        let artifact = &mut manifest["artifacts"][0];
        let tampered_path = artifact["path"].as_str().expect("artifact path").to_string();
        let digest = artifact["digest"].as_str().expect("artifact digest");
        let flipped: String = digest
            .chars()
            .map(|c| if c == '0' { '1' } else { '0' })
            .collect();
        artifact["digest"] = serde_json::Value::String(flipped);
        fs::write(&path, serde_json::to_string(&manifest).expect("serialize manifest"))
            .expect("write tampered manifest");

        let err = manifest_report(&project, &run.stderr, &tree)
            .expect_err("a tampered digest must fail verification");
        assert!(
            err.contains(&tampered_path),
            "the error must name the mismatched artifact path: {err}"
        );
    }
}
