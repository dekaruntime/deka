//! End-to-end coverage for `deka self doctor` (deka#1102): the real CLI
//! subprocess reports install state — PATH enumeration in resolution order,
//! deka/dsc pairing with how-found, project pin skew, environment overrides —
//! and exits non-zero exactly when a build would break.
//!
//! Every case runs the child with a fixture PATH (fake `deka`/`dsc` shell
//! scripts echoing fixed versions) and a temp cwd, so the assertions are
//! hermetic: they never read the test runner's own PATH or install.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Version of the binary under test, e.g. "0.53.2" — fake dsc scripts echo
/// this to build the tidy (exit 0) cases.
fn running_version() -> String {
    let output = Command::new(cli_bin())
        .arg("--version")
        .output()
        .expect("run deka --version");
    // `deka --version` writes to stdout on a TTY and stderr when piped.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.split("version ")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("parse `deka [version N]`")
        .to_string()
}

fn executable_script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

/// One fixture PATH directory holding fake deka/dsc scripts.
struct FixtureBin {
    dir: PathBuf,
}

impl FixtureBin {
    fn new(root: &Path, name: &str, deka_version: &str, dsc_version: &str) -> Self {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        executable_script(
            &dir.join("deka"),
            &format!("#!/bin/sh\necho 'deka [version {deka_version}]'\n"),
        );
        executable_script(
            &dir.join("dsc"),
            &format!("#!/bin/sh\necho 'dsc [version {dsc_version}]'\n"),
        );
        Self { dir }
    }

    fn empty(root: &Path, name: &str) -> Self {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }
}

struct DoctorRun {
    status: std::process::ExitStatus,
    output: String,
}

/// Run `deka self doctor` in a hermetic child environment: fixture PATH,
/// temp cwd, no inherited compiler-resolution overrides.
fn run_doctor(cwd: &Path, path_dirs: &[&Path], extra_env: &[(&str, &str)]) -> DoctorRun {
    let path = std::env::join_paths(path_dirs).unwrap();
    let mut command = Command::new(cli_bin());
    command
        .arg("self")
        .arg("doctor")
        .current_dir(cwd)
        .env("PATH", &path)
        .env_remove("DEKA_DSC")
        .env_remove("DEKA_NO_DSC");
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("run deka self doctor");
    DoctorRun {
        status: output.status,
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

fn write_npm_pin(project: &Path, version: &str) {
    let npm = project.join("node_modules").join("@dekaruntime").join("deka");
    fs::create_dir_all(&npm).unwrap();
    fs::write(
        npm.join("package.json"),
        format!(r#"{{"name":"@dekaruntime/deka","version":"{version}"}}"#),
    )
    .unwrap();
}

#[test]
fn doctor_lists_path_binaries_in_resolution_order_with_versions() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    // The winning dsc must match the running binary so the case stays tidy.
    let first = FixtureBin::new(root.path(), "first", &version, &version);
    let second = FixtureBin::new(root.path(), "second", "0.0.1", "0.0.1");
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(
        cwd.path(),
        &[&first.dir, &second.dir],
        &[],
    );
    assert!(run.status.success(), "tidy install must exit 0: {}", run.output);

    let first_deka = first.dir.join("deka").display().to_string();
    let second_deka = second.dir.join("deka").display().to_string();
    let first_at = run.output.find(&first_deka).expect("first deka listed");
    let second_at = run.output.find(&second_deka).expect("second deka listed");
    assert!(
        first_at < second_at,
        "PATH enumeration must follow resolution order:\n{}",
        run.output
    );
    assert!(
        run
            .output
            .contains(&format!("deka [version {version}]   <- first on PATH, wins")),
        "winner marker missing:\n{}",
        run.output
    );
    assert!(
        run.output.contains("0.0.1"),
        "shadowed version reported:\n{}",
        run.output
    );
}

#[test]
fn doctor_pairs_dsc_from_path_and_flags_shadowing_as_warning_only() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    // Two agreeing dekas: untidy (warning) but exit 0.
    let first = FixtureBin::new(root.path(), "first", &version, &version);
    let second = FixtureBin::new(root.path(), "second", &version, &version);
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(cwd.path(), &[&first.dir, &second.dir], &[]);
    assert!(run.status.success(), "shadowing alone must not fail: {}", run.output);
    assert!(
        run.output.contains(&format!("resolved: {} (via PATH)", first.dir.join("dsc").display())),
        "dsc pairing must name the PATH winner:\n{}",
        run.output
    );
    assert!(
        run.output.contains("[warning]") && run.output.contains("shadows"),
        "shadowing reported as a warning:\n{}",
        run.output
    );
}

#[test]
fn doctor_fails_on_deka_dsc_version_mismatch() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    // A dsc version that never equals the running binary's.
    let skewed = "0.0.1";
    let bin = FixtureBin::new(root.path(), "bin", &version, skewed);
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(cwd.path(), &[&bin.dir], &[]);
    assert!(!run.status.success(), "mismatch must exit non-zero: {}", run.output);
    assert!(
        run.output.contains("ERROR") && run.output.contains("mismatch"),
        "mismatch finding missing:\n{}",
        run.output
    );
}

#[test]
fn doctor_prefers_deka_dsc_and_validates_it() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let bin = FixtureBin::new(root.path(), "bin", &version, &version);
    let override_dir = FixtureBin::new(root.path(), "override", &version, &version);
    let cwd = tempfile::tempdir().unwrap();
    let override_dsc = override_dir.dir.join("dsc");

    // Valid DEKA_DSC wins over PATH and matches the running binary: exit 0.
    let run = run_doctor(
        cwd.path(),
        &[&bin.dir],
        &[("DEKA_DSC", override_dsc.to_str().unwrap())],
    );
    assert!(run.status.success(), "valid DEKA_DSC must exit 0: {}", run.output);
    assert!(
        run.output.contains(&format!("resolved: {} (via DEKA_DSC)", override_dsc.display())),
        "DEKA_DSC pairing missing:\n{}",
        run.output
    );
    assert!(
        run.output.contains("DEKA_DSC=") && run.output.contains("in play: file exists"),
        "override must be listed in the environment section:\n{}",
        run.output
    );

    // Invalid DEKA_DSC: the same error branch find_cli_dsc hits, exit 1.
    let missing = root.path().join("nope/dsc");
    let run = run_doctor(
        cwd.path(),
        &[&bin.dir],
        &[("DEKA_DSC", missing.to_str().unwrap())],
    );
    assert!(!run.status.success(), "invalid DEKA_DSC must exit non-zero: {}", run.output);
    assert!(
        run.output.contains("not a file"),
        "invalid override finding missing:\n{}",
        run.output
    );
}

#[test]
fn doctor_reports_no_dsc_resolvable_as_error() {
    let root = tempfile::tempdir().unwrap();
    let empty = FixtureBin::empty(root.path(), "empty");
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(cwd.path(), &[&empty.dir], &[]);
    assert!(!run.status.success(), "no dsc must exit non-zero: {}", run.output);
    assert!(
        run.output.contains("no dsc resolvable"),
        "missing-dsc finding missing:\n{}",
        run.output
    );
}

#[test]
fn doctor_reports_deka_no_dsc_as_disabled_warning() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let bin = FixtureBin::new(root.path(), "bin", &version, &version);
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(cwd.path(), &[&bin.dir], &[("DEKA_NO_DSC", "1")]);
    assert!(run.status.success(), "DEKA_NO_DSC is deliberate, not a failure: {}", run.output);
    assert!(
        run.output.contains("disabled (DEKA_NO_DSC is set)"),
        "disabled pairing missing:\n{}",
        run.output
    );
}

#[test]
fn doctor_fails_on_project_pin_skew_and_says_which_is_older() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let bin = FixtureBin::new(root.path(), "bin", &version, &version);
    let project = tempfile::tempdir().unwrap();
    write_npm_pin(project.path(), "0.99.0");

    let run = run_doctor(project.path(), &[&bin.dir], &[]);
    assert!(!run.status.success(), "pin skew must exit non-zero: {}", run.output);
    assert!(
        run.output.contains("node_modules/@dekaruntime/deka: 0.99.0"),
        "project pin must be reported:\n{}",
        run.output
    );
    assert!(
        run.output.contains("NEWER than") && run.output.contains("running binary is older"),
        "must say plainly which side is older:\n{}",
        run.output
    );
}

#[test]
fn doctor_passes_when_project_pin_matches_running_binary() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let bin = FixtureBin::new(root.path(), "bin", &version, &version);
    let project = tempfile::tempdir().unwrap();
    write_npm_pin(project.path(), &version);

    let run = run_doctor(project.path(), &[&bin.dir], &[]);
    assert!(run.status.success(), "matching pin must exit 0: {}", run.output);
    assert!(
        run.output.contains("no problems found") || run.output.contains("0 error(s)"),
        "summary missing:\n{}",
        run.output
    );
}

#[test]
fn doctor_without_a_project_is_a_clean_skip() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let bin = FixtureBin::new(root.path(), "bin", &version, &version);
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(cwd.path(), &[&bin.dir], &[]);
    assert!(run.status.success(), "no project must exit 0: {}", run.output);
    assert!(
        run.output.contains("no project pin found"),
        "no-project note missing:\n{}",
        run.output
    );
}
