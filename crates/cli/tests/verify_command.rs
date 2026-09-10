// deka#738 F3: `deka verify` recomputes the build manifest's artifact
// digests against the on-disk dist tree. Clean run exits 0; a tampered or
// missing artifact exits non-zero naming the path; a missing manifest or
// dist/ exits non-zero with a clear error.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn init_project(dir: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(dir)
        .output()
        .expect("run deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_verify(project: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("verify")
        .current_dir(project)
        .output()
        .expect("run deka verify");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn manifest_path(project: &Path) -> PathBuf {
    // deka#762: verify is anchored on the v2 artifact manifest in dist/, not
    // the dev-cache v1 manifest.
    project.join("dist").join("build-manifest.json")
}

fn built_project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(project.path())
        .output()
        .expect("run deka build");
    assert!(
        output.status.success(),
        "deka build failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    project
}

#[test]
fn verify_clean_tree_exits_zero() {
    let project = built_project();
    let (success, combined) = run_verify(project.path());
    assert!(success, "deka verify must exit 0 on a clean tree: {combined}");
}

#[test]
fn verify_tampered_artifact_fails_naming_path() {
    let project = built_project();
    let tampered = project.path().join("dist").join("client").join("index.html");
    assert!(tampered.is_file(), "fixture builds publish client/index.html");
    let mut bytes = fs::read(&tampered).expect("read index.html");
    bytes.extend_from_slice(b"TAMPERED");
    fs::write(&tampered, bytes).expect("tamper with index.html");

    let (success, combined) = run_verify(project.path());
    assert!(
        !success,
        "deka verify must exit non-zero when an artifact no longer matches the manifest"
    );
    assert!(
        combined.contains("dist/client/index.html"),
        "the failure must name the tampered path: {combined}"
    );
}

#[test]
fn verify_deleted_artifact_fails_naming_path() {
    let project = built_project();
    let deleted = project.path().join("dist").join("client").join("index.html");
    fs::remove_file(&deleted).expect("delete index.html");

    let (success, combined) = run_verify(project.path());
    assert!(
        !success,
        "deka verify must exit non-zero when an artifact is missing"
    );
    assert!(
        combined.contains("dist/client/index.html"),
        "the failure must name the missing path: {combined}"
    );
}

#[test]
fn verify_without_manifest_fails_clearly() {
    let project = built_project();
    fs::remove_file(manifest_path(project.path())).expect("remove manifest");

    let (success, combined) = run_verify(project.path());
    assert!(
        !success,
        "deka verify must exit non-zero without a build manifest"
    );
    assert!(
        combined.contains("build-manifest.json") && combined.contains("deka build"),
        "the error should name the manifest and point at `deka build`: {combined}"
    );
}

#[test]
fn verify_without_dist_fails_clearly() {
    let project = built_project();
    fs::remove_dir_all(project.path().join("dist")).expect("remove dist");

    let (success, combined) = run_verify(project.path());
    assert!(
        !success,
        "deka verify must exit non-zero without a dist tree"
    );
    assert!(
        combined.contains("dist") && combined.contains("deka build"),
        "the error should name dist/ and point at `deka build`: {combined}"
    );
}
