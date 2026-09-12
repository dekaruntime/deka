//! deka#720: deterministic-artifact contract harness for `deka build`.
//! Shared by `build_artifacts.rs` via `#[path]`.
//!
//! Each fixture lives in `tests/fixtures/build/<name>/`:
//!
//! ```text
//! project/             the deka project source (copied to a tempdir)
//! expected/
//!   stderr.txt         expected stderr after path normalization, including
//!                      diagnostics
//!   fail               marker: build must fail; dist/ must not exist
//! ```
//!
//! The successful-build fixtures (static-site, static-params, request-time)
//! and their tree/bytes/manifest machinery were removed with the
//! paused-framework teardown (deka#881): they pinned JSX-rendered prerender
//! HTML, island assets, and route CSS, none of which `deka build` emits
//! anymore. (App-router projects still build and publish dist/ + manifests;
//! the paused `ui/*` imports in the serve entry stay unresolved at serve
//! time until the framework returns in dsc, RFD 60.) What remains pins the
//! pre-publication failure boundary (route collisions).
//!
//! `DEKA_BLESS=1` regenerates the expected stderr from actual output — the
//! only way expected bytes may change.

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

/// The build plan version the installed dsc emits. Probed once per test
/// process.
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
    let mut out = stderr.replace(project_canonical.to_str().expect("utf8 path"), "<project>");
    if project != project_canonical {
        out = out.replace(project.to_str().expect("utf8 path"), "<project>");
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

/// A small line-based unified diff (3 lines of context) for readable
/// mismatch reports. Build artifacts are at most a few thousand lines, so
/// the quadratic LCS table is fine; larger inputs fall back to a plain dump.
pub(crate) fn unified_diff(expected: &str, actual: &str) -> String {
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
        let expected = self.expected();
        if plan_version < 2 && expected.join("stderr.v1.txt").is_file() {
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

fn bless(fixture: &Fixture, stderr_name: &str, run: &BuildRun, stderr: &str) {
    assert!(
        run.stdout.is_empty(),
        "cannot bless `{}`: stdout must be empty (stdio convention), got {:?}",
        fixture.name,
        run.stdout
    );
    assert!(
        !run.success,
        "cannot bless `{}`: build succeeded but the fixture expects failure",
        fixture.name
    );
    let expected_dir = fixture.expected();
    fs::create_dir_all(&expected_dir).expect("mkdir expected");
    fs::write(expected_dir.join(stderr_name), stderr).expect("write blessed stderr");
    eprintln!(
        "DEKA_BLESS: regenerated {} for fixture `{}`",
        stderr_name, fixture.name
    );
}

/// Run one build fixture end to end and assert the failure contract: the
/// build exits non-zero with the expected diagnostic and publishes no dist/.
/// Panics with a path-labeled report on any mismatch.
pub fn check_fixture(name: &str) {
    let fixture = Fixture::load(name);
    let plan_version = dsc_plan_version();
    let stderr_name = fixture.stderr_file(plan_version);
    let expects_failure = fixture.expects_failure(plan_version);
    eprintln!(
        "fixture `{name}`: dsc plan v{plan_version}, expecting {}, stderr `{stderr_name}`",
        if expects_failure {
            "failure"
        } else {
            "success"
        }
    );

    let tmp = tempfile::tempdir().expect("create temp project dir");
    let project = tmp.path().join("project");
    copy_dir(&fixture.root.join("project"), &project);
    let project_canonical = fs::canonicalize(&project).expect("canonicalize project");

    let run = run_build(&project);
    let stderr = normalize_stderr(&project, &project_canonical, &run.stderr);

    if std::env::var("DEKA_BLESS").ok().as_deref() == Some("1") {
        bless(&fixture, &stderr_name, &run, &stderr);
        return;
    }

    let mut problems: Vec<String> = Vec::new();
    if !run.stdout.is_empty() {
        problems.push(format!("[stdout] expected empty, got:\n{}", run.stdout));
    }
    if run.success != !expects_failure {
        problems.push(format!(
            "[exit] expected {}, build {}\nstderr:\n{}",
            if expects_failure {
                "failure"
            } else {
                "success"
            },
            if run.success { "succeeded" } else { "failed" },
            stderr
        ));
    }
    let expected_stderr =
        fs::read_to_string(fixture.expected().join(&stderr_name)).unwrap_or_else(|err| {
            panic!("fixture `{name}`: cannot read expected/{stderr_name}: {err}")
        });
    if stderr != expected_stderr {
        problems.push(format!(
            "[stderr] mismatch vs expected/{stderr_name}\n{}",
            unified_diff(&expected_stderr, &stderr)
        ));
    }

    if expects_failure && project.join("dist").exists() {
        problems.push("[tree] a failed build must not publish dist/".to_string());
    }

    if !problems.is_empty() {
        let mut report = format!("fixture `{name}` FAILED (dsc plan v{plan_version})\n\n");
        report.push_str(&problems.join("\n\n"));
        report.push_str("\n\nTo accept intentional output changes, rerun with DEKA_BLESS=1.");
        panic!("{report}");
    }
}
