// Regression test for deka#5: `deka build` must fail closed (non-zero exit,
// no dist/ output) when the web project's source under app/ does not
// compile -- matching what `deka check` already reports for that file.
//
// `deka build` prefers default dsc emit (`dsc --outdir`), falling back to
// per-tree `dsc transpile <dir> --out`, then copies host static files.
// Raw app/ .ds is not the server product.

use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

// Pin the source app-router shape independently of the static init template.
#[path = "support/app_router.rs"]
mod app_router;
use app_router::init_project;

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
        project.path().join("app").join("page.dsx"),
        "export function f(): int { return\n",
    )
    .expect("write invalid app/page.dsx");

    // Sanity: `deka check` rejects this file on its own, confirming the
    // fixture is genuinely invalid and not an environment quirk.
    let check = Command::new(cli_bin())
        .args(["check", "app/page.dsx"])
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
        combined.contains("page.dsx"),
        "build failure diagnostic should name the offending file: {combined}"
    );

    // Fail-closed: no dist/ output should be produced when validation fails.
    assert!(
        !project.path().join("dist").exists(),
        "deka build should not write dist/ output when source fails to compile"
    );
}
