// Regression test for deka#5: `deka build` must fail closed (non-zero exit,
// no dist/ output) when the web project's source under app/ does not
// compile -- matching what `deka check` already reports for that file.
//
// Root cause (see runtime/crates/cli/src/cli/build.rs): the web-project
// build path only ever compiled `serve.entry`, and only when the entry
// used a hydration component; every other .ds file under app/, including
// the entry itself in the common non-hydration case, was copied into
// dist/server/app as raw, unvalidated bytes via copy_dir_recursive. A
// project with syntactically invalid source therefore "built" successfully.

use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Scaffolds a fresh web project into `dir` via `deka init`, exactly the way
/// a human would (per the issue's reproduction), and asserts the scaffold
/// succeeded.
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
    assert!(
        dir.join("deka.json").is_file(),
        "deka init should scaffold deka.json"
    );
    assert!(
        dir.join("deka.lock").is_file(),
        "deka init should scaffold deka.lock"
    );
    assert!(
        dir.join("app").join("main.ds").is_file(),
        "deka init should scaffold app/main.ds"
    );
    assert!(
        dir.join("public").join("index.html").is_file(),
        "deka init should scaffold public/index.html"
    );
}

fn run_build(dir: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(dir)
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

#[test]
fn build_exits_nonzero_on_invalid_source() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    // Same invalid snippet as the issue's reproduction: unterminated
    // function body, which `deka check` correctly rejects (exit 1).
    fs::write(
        project.path().join("app").join("main.ds"),
        "export function f(): int { return\n",
    )
    .expect("write invalid app/main.ds");

    // Sanity: `deka check` rejects this file on its own, confirming the
    // fixture is genuinely invalid and not an environment quirk.
    let check = Command::new(cli_bin())
        .args(["check", "app/main.ds"])
        .current_dir(project.path())
        .output()
        .expect("run deka check");
    assert!(
        !check.status.success(),
        "fixture should be rejected by `deka check`; test fixture is not actually invalid"
    );

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must exit non-zero on invalid source under app/, got success. output: {combined}"
    );
    assert!(
        combined.contains("main.ds"),
        "build failure diagnostic should name the offending file: {combined}"
    );

    // Fail-closed: no dist/ output should be produced when validation fails.
    assert!(
        !project.path().join("dist").exists(),
        "deka build should not write dist/ output when source fails to compile"
    );
}

#[test]
fn build_exits_zero_on_valid_source() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should exit 0 for the unmodified `deka init` scaffold: {combined}"
    );

    assert!(
        project.path().join("dist").join("client").join("index.html").is_file(),
        "successful build should produce dist/client/index.html: {combined}"
    );
    assert!(
        project.path().join("dist").join("server").join("app").join("main.ds").is_file(),
        "successful build should copy app/ into dist/server/app: {combined}"
    );
}
