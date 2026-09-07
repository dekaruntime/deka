use std::fs;
use std::path::Path;
use std::process::Command;

use deka_host::integrity::compute_package_integrity;

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

/// Project with `io` installed under `ds_modules/` but absent from deka.lock.
fn project_with_installed_module(manifest: &str) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("deka.json"), manifest).expect("manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lockfile");

    let module_dir = project.path().join("ds_modules").join("@deka").join("io");
    fs::create_dir_all(&module_dir).expect("module dir");
    fs::write(
        module_dir.join("index.ds"),
        "export fn echo(value: string) string {\n    return value\n}\n",
    )
    .expect("module source");

    fs::write(
        project.path().join("main.ds"),
        "import { echo } from \"io\"\n\nexport fn handler() string {\n    return echo(\"hi\")\n}\n",
    )
    .expect("entry");
    project
}

fn lock_installed_module(project: &Path) {
    let module_dir = project.join("ds_modules").join("@deka").join("io");
    let integrity = compute_package_integrity(&module_dir).expect("package integrity");
    fs::write(
        project.join("deka.lock"),
        serde_json::json!({
            "lockfileVersion": 1,
            "packages": {
                "@deka/io": [
                    "@deka/io@0.0.0",
                    "local:@deka/io",
                    {
                        "moduleGraph": { "hash": integrity.module_graph },
                        "fsGraph": { "hash": integrity.fs_graph }
                    },
                    ""
                ]
            }
        })
        .to_string(),
    )
    .expect("write package lock entry");
}

fn run_entry(project: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .args(["run", "main.ds"])
        .current_dir(project)
        .output()
        .expect("run through the CLI");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn runtime_rejects_an_installed_stdlib_import_without_a_lock_entry() {
    let project = project_with_installed_module("{\"name\":\"gate\"}\n");
    let (success, combined) = run_entry(project.path());
    assert!(
        !success && combined.contains("has no deka.lock entry"),
        "installed import without a lock entry must be rejected: {combined}"
    );
}

#[test]
fn runtime_accepts_the_same_project_once_the_import_is_declared() {
    let project = project_with_installed_module(
        "{\"name\":\"gate\",\"dependencies\":{\"@deka/io\":\"*\"}}\n",
    );
    lock_installed_module(project.path());
    let (success, combined) = run_entry(project.path());
    assert!(
        success,
        "declared and locked dependency must execute: {combined}"
    );
}

#[test]
fn runtime_rejects_a_project_with_no_lockfile() {
    let project = project_with_installed_module(
        "{\"name\":\"gate\",\"dependencies\":{\"@deka/io\":\"*\"}}\n",
    );
    fs::remove_file(project.path().join("deka.lock")).expect("remove lockfile");
    let (success, combined) = run_entry(project.path());
    assert!(
        !success && combined.contains("deka.lock"),
        "a project without a lockfile must be rejected: {combined}"
    );
}
