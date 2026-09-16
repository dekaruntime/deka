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
    run_entry_with_env(project, &[])
}

fn run_entry_with_env(project: &Path, envs: &[(&str, &str)]) -> (bool, String) {
    let mut command = Command::new(cli_bin());
    command.args(["run", "main.ds"]).current_dir(project);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("run through the CLI");
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
    let project = project_with_installed_module(
        "{\"name\":\"gate\",\"dependencies\":{\"@deka/io\":\"*\"}}\n",
    );
    let (success, combined) = run_entry(project.path());
    assert!(
        !success && combined.contains("has no deka.lock entry"),
        "installed import without a lock entry must be rejected: {combined}"
    );
}

#[test]
fn runtime_rejects_integrity_mismatch() {
    let project = project_with_installed_module(
        "{\"name\":\"gate\",\"dependencies\":{\"@deka/io\":\"*\"}}\n",
    );
    lock_installed_module(project.path());
    fs::write(
        project
            .path()
            .join("ds_modules")
            .join("@deka")
            .join("io")
            .join("index.ds"),
        "export fn echo(value: string) string {\n    return \"tampered\"\n}\n",
    )
    .expect("tamper package");
    let (success, combined) = run_entry(project.path());
    let lower = combined.to_ascii_lowercase();
    assert!(
        !success
            && (lower.contains("integrity mismatch") || lower.contains("does not match deka.lock")),
        "tampered package tree must fail fsGraph integrity: {combined}"
    );
}

#[test]
fn runtime_rejects_undeclared_third_party_bare_import() {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("deka.json"), "{\"name\":\"gate\"}\n").expect("manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lockfile");
    let module_dir = project.path().join("ds_modules").join("@acme").join("tool");
    fs::create_dir_all(&module_dir).expect("module dir");
    fs::write(
        module_dir.join("index.ds"),
        "export fn ping() string {\n    return \"ok\"\n}\n",
    )
    .expect("module source");
    fs::write(
        project.path().join("main.ds"),
        "import { ping } from \"@acme/tool\"\n\nexport fn handler() string {\n    return ping()\n}\n",
    )
    .expect("entry");
    let (success, combined) = run_entry(project.path());
    let lower = combined.to_ascii_lowercase();
    assert!(
        !success && (lower.contains("not declared") || lower.contains("missing from deka.lock")),
        "undeclared third-party import must fail closed: {combined}"
    );
}

#[test]
fn runtime_rejects_undeclared_unknown_deka_scope_import() {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("deka.json"), "{\"name\":\"gate\"}\n").expect("manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lockfile");
    let module_dir = project
        .path()
        .join("ds_modules")
        .join("@deka")
        .join("not-a-stdlib-package");
    fs::create_dir_all(&module_dir).expect("module dir");
    fs::write(
        module_dir.join("index.ds"),
        "export fn ping() string {\n    return \"ok\"\n}\n",
    )
    .expect("module source");
    fs::write(
        project.path().join("main.ds"),
        "import { ping } from \"@deka/not-a-stdlib-package\"\n\nexport fn handler() string {\n    return ping()\n}\n",
    )
    .expect("entry");
    let (success, combined) = run_entry(project.path());
    let lower = combined.to_ascii_lowercase();
    assert!(
        !success && (lower.contains("not declared") || lower.contains("missing from deka.lock")),
        "unknown @deka/* is not stdlib and must be declared: {combined}"
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

// deka#229: `DEKA_MODULE_ROOT` used to short-circuit the pool ESM loader's
// copy of the project gate — its mere presence returned Ok, skipping the
// lockfile requirement, integrity verification, and every module-presence
// check. The gate is a shared explicit-parameter API now and reads no process
// environment: offering a fully valid project through the env var must not
// rescue an integrity-tampered one. (The layout is deliberately one that dsc
// itself accepts — declared, locked, installed — so only the gate can reject
// it; a missing-lockfile layout would also be caught by the transpile
// wrapper's own lockfile requirement and prove nothing about the gate.)
#[test]
fn env_module_root_cannot_bypass_the_project_gate() {
    let project = project_with_installed_module(
        "{\"name\":\"gate\",\"dependencies\":{\"@deka/io\":\"*\"}}\n",
    );
    lock_installed_module(project.path());
    fs::write(
        project
            .path()
            .join("ds_modules")
            .join("@deka")
            .join("io")
            .join("index.ds"),
        "export fn echo(value: string) string {\n    return \"tampered\"\n}\n",
    )
    .expect("tamper package");

    // Everything the old bypass would have pointed at: a complete project
    // with manifest, lockfile, and a locked, installed module.
    let offered = project_with_installed_module(
        "{\"name\":\"offered\",\"dependencies\":{\"@deka/io\":\"*\"}}\n",
    );
    lock_installed_module(offered.path());
    let offered_root = offered.path().to_string_lossy().into_owned();

    let (success, combined) =
        run_entry_with_env(project.path(), &[("DEKA_MODULE_ROOT", &offered_root)]);
    let lower = combined.to_ascii_lowercase();
    assert!(
        !success
            && (lower.contains("integrity mismatch") || lower.contains("does not match deka.lock")),
        "a valid DEKA_MODULE_ROOT must not waive fsGraph integrity: {combined}"
    );
}

// The flip side of the same pin: a garbage module root must not change the
// outcome for a valid project either. Resolution is a function of the
// project on disk, never of the ambient environment.
#[test]
fn declared_project_runs_with_garbage_module_root_env() {
    let project = project_with_installed_module(
        "{\"name\":\"gate\",\"dependencies\":{\"@deka/io\":\"*\"}}\n",
    );
    lock_installed_module(project.path());
    let (success, combined) =
        run_entry_with_env(project.path(), &[("DEKA_MODULE_ROOT", "/not/a/project")]);
    assert!(
        success,
        "declared and locked dependency must execute with a garbage DEKA_MODULE_ROOT: {combined}"
    );
}
