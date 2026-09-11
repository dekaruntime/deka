//! The testsuite runner must enforce the same gates in `--json` mode as in
//! human-readable mode. `--json` used to print results and `exit(0)` before
//! the ratchet and reconciliation checks ran, so an automated caller could
//! see success while fixtures were mismatched (dekaruntime/deka#539).
//!
//! This test builds a hermetic one-fixture corpus (via `run.mjs --root`),
//! runs it through both output modes, and asserts the exit statuses agree:
//! a failing corpus exits non-zero in BOTH modes, and the same corpus exits
//! zero in BOTH modes once the fixture is listed in expected-failures.txt.
//! The corpus lives in a tempdir so the committed fixtures are untouched.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn runner() -> PathBuf {
    repo_root().join("scripts").join("testsuite-run.mjs")
}

/// The runner executes fixtures with `target/release/cli` (or DEKA_NATIVE).
/// CI builds the release CLI before running the workspace tests (deka#521);
/// locally, build it with `cargo build --release -p cli` first.
fn release_cli() -> PathBuf {
    let cli = repo_root().join("target").join("release").join("cli");
    assert!(
        cli.exists(),
        "testsuite runner needs target/release/cli; run `cargo build --release -p cli` first"
    );
    cli
}

fn bun() -> String {
    let output = Command::new("which")
        .arg("bun")
        .output()
        .expect("spawn which");
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!path.is_empty(), "bun is required on PATH to run run.mjs");
    path
}

fn write_corpus(root: &Path, listed: bool) {
    let fixture = root.join("zgate").join("negative");
    fs::create_dir_all(&fixture).expect("fixture dir");
    // A compile error in a fixture that declares itself as passing
    // (.pass.ds): expected pass, actual fail — a genuine gate failure.
    fs::write(
        fixture.join("negative.pass.ds"),
        "const x: number = \"hello\"\n",
    )
    .expect("source");
    fs::write(
        fixture.join("negative.json"),
        r#"{"title": "json gate probe", "stage": "typecheck", "hosts": ["native"]}"#,
    )
    .expect("metadata");
    let ratchet = if listed { "zgate-negative\n" } else { "" };
    fs::write(root.join("expected-failures.txt"), ratchet).expect("ratchet");
}

fn write_registry_fixture(root: &Path) {
    let fixture = root.join("zgate").join("registry");
    fs::create_dir_all(&fixture).expect("fixture dir");
    fs::write(
        fixture.join("registry.pass.ds"),
        "console.log(\"never runs\")\n",
    )
    .expect("source");
    fs::write(
        fixture.join("registry.json"),
        r#"{"title":"registry fault probe","stage":"run","hosts":["native"],"packages":["@deka/time"]}"#,
    )
    .expect("metadata");
    fs::write(root.join("expected-failures.txt"), "").expect("ratchet");
}

#[cfg(unix)]
fn unavailable_registry_cli(root: &Path) -> PathBuf {
    let cli = root.join("unavailable-registry-cli");
    fs::write(
        &cli,
        "#!/bin/sh\necho 'failed to install @deka/time from deka.gg: failed to contact deka.gg registry for @deka/time: connection refused' >&2\nexit 1\n",
    )
    .expect("fake cli");
    let mut permissions = fs::metadata(&cli).expect("fake cli metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&cli, permissions).expect("make fake cli executable");
    cli
}

fn run_mode(corpus: &Path, json: bool) -> std::process::ExitStatus {
    let mut cmd = Command::new(bun());
    cmd.arg(runner())
        .arg("--root")
        .arg(corpus)
        .arg("--filter")
        .arg("zgate-negative")
        .env("DEKA_NATIVE", release_cli());
    if json {
        cmd.arg("--json");
    }
    cmd.status().expect("spawn run.mjs")
}

#[test]
fn json_mode_enforces_the_same_exit_status_as_text_mode() {
    let corpus = tempfile::tempdir().expect("corpus");

    // Unlisted failing fixture: both modes must report failure.
    write_corpus(corpus.path(), false);
    let text = run_mode(corpus.path(), false);
    let json = run_mode(corpus.path(), true);
    assert_eq!(
        text.code(),
        json.code(),
        "--json and text mode disagree on exit status for a failing corpus"
    );
    assert!(
        matches!(text.code(), Some(1)),
        "a failing corpus must exit 1, got {:?}",
        text.code()
    );

    // Listed in expected-failures.txt (the ratchet): both modes must accept it.
    write_corpus(corpus.path(), true);
    let text = run_mode(corpus.path(), false);
    let json = run_mode(corpus.path(), true);
    assert_eq!(
        text.code(),
        json.code(),
        "--json and text mode disagree on exit status for a ratcheted corpus"
    );
    assert!(
        matches!(text.code(), Some(0)),
        "a fully ratcheted corpus must exit 0, got {:?}",
        text.code()
    );
}

#[cfg(unix)]
#[test]
fn registry_fetch_fault_is_blocked_and_visible_not_a_fixture_failure() {
    let corpus = tempfile::tempdir().expect("corpus");
    write_registry_fixture(corpus.path());
    let cli = unavailable_registry_cli(corpus.path());

    let output = Command::new(bun())
        .arg(runner())
        .arg("--root")
        .arg(corpus.path())
        .arg("--filter")
        .arg("zgate-registry")
        .env("DEKA_NATIVE", cli)
        .output()
        .expect("spawn run.mjs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(
        output.status.code(),
        Some(2),
        "blocked run output: {stdout}"
    );
    assert!(
        stdout.contains("⊘ zgate-registry"),
        "blocked fixture was not named: {stdout}"
    );
    assert!(
        stdout.contains("0 fail · 1 blocked"),
        "blocked count missing: {stdout}"
    );
    assert!(
        stdout.contains("failed to contact deka.gg registry"),
        "install cause missing: {stdout}"
    );
}
