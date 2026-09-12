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
fn build_help_does_not_build() {
    // deka#750: `deka build --help` used to fall through to the build handler
    // (which needs a project and would fail or write dist). Help must print
    // and exit 0 without touching the filesystem.
    let project = tempfile::tempdir().expect("create temp project dir");
    let output = Command::new(cli_bin())
        .args(["build", "--help"])
        .current_dir(project.path())
        .output()
        .expect("run deka build --help");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "deka build --help must exit 0: {combined}");
    assert!(combined.contains("build"), "help must mention build: {combined}");
    assert!(
        !project.path().join("dist").exists(),
        "deka build --help must not write dist"
    );
}

/// Scaffold a minimal non-app-router web project (`serve.entry` style): the
/// `deka init` scaffold is an app-router project, and app-router builds are
/// paused with the framework (deka#881), so this test scaffolds the plain
/// serve-entry form directly.
fn scaffold_serve_entry_project(dir: &Path) {
    fs::write(
        dir.join("deka.json"),
        "{\n  \"name\": \"emit-src\",\n  \"type\": \"serve\",\n  \"serve\": { \"mode\": \"ds\", \"entry\": \"app/main.ds\" },\n  \"security\": { \"allow\": {}, \"deny\": {}, \"prompt\": true }\n}\n",
    )
    .expect("write deka.json");
    fs::write(
        dir.join("deka.lock"),
        "{\n  \"lockfileVersion\": 1,\n  \"packages\": {}\n}\n",
    )
    .expect("write deka.lock");
    fs::create_dir_all(dir.join("app")).expect("mkdir app");
    fs::write(
        dir.join("app").join("main.ds"),
        "export fn handle() string {\n  return \"hello\"\n}\n",
    )
    .expect("write app/main.ds");
    fs::create_dir_all(dir.join("public")).expect("mkdir public");
    fs::write(
        dir.join("index.html"),
        "<!doctype html>\n<html><head></head><body><div id=\"app\"></div></body></html>\n",
    )
    .expect("write index.html");
}

#[test]
fn build_emits_src_one_to_one() {
    let project = tempfile::tempdir().expect("create temp project dir");
    scaffold_serve_entry_project(project.path());
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

    let dist_src = project.path().join("dist").join("server").join("src");
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

#[test]
fn build_stops_when_a_build_block_returns_err() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app/page.dsx"),
        r#"const title: string = build {
  return Err("content source is unavailable")
}

export fn Page() {
  return <section><h1>{title}</h1></section>
}
"#,
    )
    .expect("write failing build page");

    let (success, combined) = run_build(project.path());
    assert!(!success, "Result.Err must stop deka build: {combined}");
    assert!(
        combined.contains("build `title` (at ")
            && combined.contains("page.dsx:1:23) failed: content source is unavailable"),
        "build error must link the source location and preserve Result.Err text: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "failed build entries must not promote dist output"
    );
}


