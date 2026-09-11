// deka#719, the property to test hardest: a killed build must leave `dist/`
// in exactly one consistent state — entirely the previous build or entirely
// the new one — never a mixture of the two.
//
// These tests run a REAL `deka build` as a child process, SIGKILL it at a
// precise point of the pipeline, and assert the on-disk state:
//
// - `killed_mid_staging_*` kills after staging artifacts exist but before
//   publication begins: `dist/` must be byte-identical to the previous
//   build.
// - `killed_mid_publish_*` kills inside the publication transaction (the
//   move-aside rename has happened, the promotion rename has not — a
//   test-only `DEKA_TEST_PUBLISH_PAUSE_MS` widens that window): the previous
//   output must survive whole at `dist.prev-*`, and the next build must roll
//   it forward even when that build itself fails.
//
// Real dsc required, same convention as the other build suites: `DEKA_DSC`,
// then `dsc` on PATH.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
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
        "tests require dsc (set DEKA_DSC or put dsc on PATH)"
    );
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

/// Scaffolds a fresh web project into `dir` via `deka init`, exactly the way
/// a human would, and asserts the scaffold succeeded.
fn init_project(dir: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(dir)
        .env("DEKA_DSC", real_dsc())
        .output()
        .expect("run deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_page(project: &Path, marker: &str) {
    fs::write(
        project.join("app").join("page.dsx"),
        format!("export fn Page() {{\n    return <main>{marker}</main>;\n}}\n"),
    )
    .expect("write app/page.dsx");
}

fn run_build(dir: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(dir)
        .env("DEKA_DSC", real_dsc())
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// Walk `dir` into a relative-path -> bytes map for tree comparison.
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

fn dist_snapshot(project: &Path) -> BTreeMap<String, Vec<u8>> {
    tree_snapshot(&project.join("dist"))
}

/// Paths directly under `project` whose name starts with `prefix`.
fn root_entries_with_prefix(project: &Path, prefix: &str) -> Vec<PathBuf> {
    fs::read_dir(project)
        .expect("read project root")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .expect("file name")
                .to_string_lossy()
                .starts_with(prefix)
        })
        .collect()
}

/// Spawn `deka build` with stdout/stderr captured to `<dir>/interrupt.log`.
fn spawn_build(dir: &Path, extra_env: &[(&str, &str)]) -> Child {
    let log = fs::File::create(dir.join("interrupt.log")).expect("create interrupt log");
    let mut command = Command::new(cli_bin());
    command
        .arg("build")
        .current_dir(dir)
        .env("DEKA_DSC", real_dsc())
        .stdout(Stdio::from(log.try_clone().expect("clone interrupt log")))
        .stderr(Stdio::from(log));
    for (key, value) in extra_env {
        command.env(key, value);
    }
    command.spawn().expect("spawn deka build")
}

fn build_log(dir: &Path) -> String {
    fs::read_to_string(dir.join("interrupt.log")).unwrap_or_default()
}

/// Poll `condition` until it holds or the deadline passes. Returns whether
/// the condition was observed.
fn wait_for(deadline_secs: u64, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(deadline_secs);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}

fn kill_and_reap(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn killed_mid_staging_build_leaves_prior_dist_entirely_intact() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_page(project.path(), "first edition");
    let (ok, output) = run_build(project.path());
    assert!(ok, "initial build must succeed: {output}");
    let prior = dist_snapshot(project.path());
    assert!(
        !prior.is_empty(),
        "initial build must publish a non-empty dist"
    );

    // Second build, killed after the staging tree exists (some new artifacts
    // written under .deka-dist-stage/dist) but before publication begins.
    write_page(project.path(), "second edition");
    let mut child = spawn_build(project.path(), &[]);
    let observed = wait_for(90, || project.path().join(".deka-dist-stage").join("dist").exists());
    if !observed {
        kill_and_reap(child);
        panic!(
            "staged dist never appeared; build log:\n{}",
            build_log(project.path())
        );
    }
    assert!(
        project.path().join("dist").exists(),
        "dist must still be in place while staging is in progress"
    );
    kill_and_reap(child);

    assert_eq!(
        dist_snapshot(project.path()),
        prior,
        "a build killed mid-staging must leave dist byte-identical to the previous build"
    );
    assert!(
        root_entries_with_prefix(project.path(), "dist.prev-").is_empty(),
        "a build killed before publication must not have moved dist aside"
    );
}

#[test]
fn killed_mid_publish_recovers_prior_dist_on_next_build() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    write_page(project.path(), "first edition");
    let (ok, output) = run_build(project.path());
    assert!(ok, "initial build must succeed: {output}");
    let first = dist_snapshot(project.path());

    write_page(project.path(), "second edition");
    let (ok, output) = run_build(project.path());
    assert!(ok, "second build must succeed: {output}");
    let second = dist_snapshot(project.path());
    assert_ne!(
        first, second,
        "rebuilding a changed page must produce different dist bytes"
    );

    // Third build, killed inside the publication transaction: the move-aside
    // rename has happened (dist.prev-* exists), the promotion rename has not
    // (the pause is still sleeping). The previous output must survive whole
    // at dist.prev-* and dist/ must not exist in a mixed state.
    write_page(project.path(), "third edition");
    let mut child = spawn_build(
        project.path(),
        &[("DEKA_TEST_PUBLISH_PAUSE_MS", "3000")],
    );
    let observed = wait_for(90, || {
        !root_entries_with_prefix(project.path(), "dist.prev-").is_empty()
    });
    if !observed {
        kill_and_reap(child);
        panic!(
            "dist.prev-* never appeared; build log:\n{}",
            build_log(project.path())
        );
    }
    // We must be inside the window: the staged tree has not been promoted yet.
    assert!(
        project.path().join(".deka-dist-stage").join("dist").exists(),
        "staged dist must still be waiting for the promotion rename"
    );
    kill_and_reap(child);

    let dist = project.path().join("dist");
    assert!(!dist.exists(), "a crash between the renames leaves no dist/");
    let prevs = root_entries_with_prefix(project.path(), "dist.prev-");
    assert_eq!(
        prevs.len(),
        1,
        "exactly one moved-aside backup may exist, got: {prevs:?}"
    );
    assert_eq!(
        tree_snapshot(&prevs[0]),
        second,
        "the moved-aside dist must be entirely the previous build, byte for byte"
    );

    // The next build rolls the interrupted publish forward BEFORE doing any
    // of its own work — and must do so even when the new build itself fails.
    // Break the page so the build dies at compile time:
    fs::write(
        project.path().join("app").join("page.dsx"),
        "this is not dekascript at all\n",
    )
    .expect("break page.dsx");
    let (ok, output) = run_build(project.path());
    assert!(!ok, "the broken build must fail: {output}");
    assert_eq!(
        dist_snapshot(project.path()),
        second,
        "after recovery plus a failed rebuild, dist must still be exactly the last good build"
    );
    assert!(
        root_entries_with_prefix(project.path(), "dist.prev-").is_empty(),
        "a consumed backup must not linger"
    );

    // And the pipeline still works end to end afterwards: a good build
    // publishes the new edition whole.
    write_page(project.path(), "third edition");
    let (ok, output) = run_build(project.path());
    assert!(ok, "build after recovery must succeed: {output}");
    let third = dist_snapshot(project.path());
    assert_ne!(third, second, "the third build must publish new bytes");
    let index = third
        .get("client/index.html")
        .expect("third build publishes a client index");
    assert!(
        String::from_utf8_lossy(index).contains("third edition"),
        "published index must be the third edition whole, not a mixture"
    );
}
