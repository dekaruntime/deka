use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;

// ---------------------------------------------------------------------------
// Project gate, exercised across the real process boundary.
//
// deka#430: `ensure_project_layout` had green unit tests and was unreachable —
// the v2 pipeline pre-compiles the module graph and `load_ds_source` returned
// before the gate call. An in-process test of the validator cannot catch that,
// so these drive the actual CLI.
// ---------------------------------------------------------------------------

/// Project with `io` installed under `ds_modules/` but absent from `deka.json`.
fn project_with_installed_module(manifest: &str) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("deka.json"), manifest).expect("manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lockfile");

    let module_dir = project.path().join("ds_modules").join("@deka").join("io");
    fs::create_dir_all(&module_dir).expect("module dir");
    fs::write(
        module_dir.join("index.ds"),
        "export fn echo(value: string): string {\n    return value\n}\n",
    )
    .expect("module source");

    fs::write(
        project.path().join("main.ds"),
        "import { echo } from \"io\"\n\nexport fn handler(): string {\n    return echo(\"hi\")\n}\n",
    )
    .expect("entry");
    project
}

fn run_entry(project: &Path) -> String {
    let output = Command::new(cli_bin())
        .args(["run", "main.ds"])
        .current_dir(project)
        .output()
        .expect("run through the CLI");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn runtime_rejects_an_installed_but_undeclared_stdlib_import() {
    let project = project_with_installed_module("{\"name\":\"gate\"}\n");
    let combined = run_entry(project.path());
    assert!(
        combined.contains("not declared in deka.json"),
        "installed-but-undeclared import must be rejected: {combined}"
    );
}

#[test]
fn runtime_accepts_the_same_project_once_the_import_is_declared() {
    let project =
        project_with_installed_module("{\"name\":\"gate\",\"dependencies\":{\"@deka/io\":\"*\"}}\n");
    let combined = run_entry(project.path());
    assert!(
        !combined.contains("not declared in deka.json"),
        "declaring the dependency must clear the gate: {combined}"
    );
}

#[test]
fn runtime_rejects_a_project_with_no_lockfile() {
    let project = project_with_installed_module("{\"name\":\"gate\",\"dependencies\":{\"@deka/io\":\"*\"}}\n");
    fs::remove_file(project.path().join("deka.lock")).expect("remove lockfile");
    let combined = run_entry(project.path());
    assert!(
        combined.contains("deka.lock"),
        "a project without a lockfile must be rejected: {combined}"
    );
}
