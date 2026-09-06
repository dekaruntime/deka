//! `deka build` compiles through dsc, then copies host static files.

use std::fs;
use std::path::Path;
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
fn build_emits_src_one_to_one() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let src = project.path().join("src");
    fs::create_dir_all(src.join("nested")).expect("mkdir src/nested");
    fs::write(src.join("util.ds"), "export const n = 1;\n").expect("write src/util.ds");
    fs::write(src.join("notes.txt"), "keep me\n").expect("write src/notes.txt");
    fs::write(src.join("nested").join("readme.md"), "verbatim\n")
        .expect("write src/nested/readme.md");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with a no-magic src/ tree: {combined}"
    );

    let dist_src = project.path().join("dist").join("src");
    let util_js = fs::read_to_string(dist_src.join("util.js")).expect("read dist/src/util.js");
    assert!(
        util_js.contains("export const n") || util_js.contains("n = 1"),
        "src/ .ds must compile via dsc into dist/src: {util_js}"
    );
    assert!(
        !dist_src.join("util.ds").exists(),
        "src/ .ds must not be copied raw into dist/src"
    );
    assert_eq!(
        fs::read_to_string(dist_src.join("notes.txt")).expect("read notes"),
        "keep me\n"
    );
    assert_eq!(
        fs::read_to_string(dist_src.join("nested").join("readme.md")).expect("read readme"),
        "verbatim\n"
    );
}

#[test]
fn build_missing_dsc_is_hard_error() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(project.path())
        .env("DEKA_NO_DSC", "1")
        .env_remove("DEKA_DSC")
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "missing dsc must fail the build: {combined}"
    );
    assert!(
        combined.contains("dsc is required"),
        "error should say dsc is required: {combined}"
    );
    assert!(
        combined.contains("DEKA_DSC"),
        "error should mention DEKA_DSC: {combined}"
    );
    assert!(
        combined.contains("https://deka.gg/install") || combined.contains("install"),
        "error should include install guidance: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "missing dsc must not write dist/"
    );
}
