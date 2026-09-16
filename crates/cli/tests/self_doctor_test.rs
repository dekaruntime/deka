//! End-to-end coverage for `deka self doctor` (deka#1102): the real CLI
//! subprocess reports install state — PATH enumeration in resolution order,
//! per-path dsc pairing (CLI dispatch vs sibling-only isolate compiles),
//! project pin skew, environment overrides — and exits non-zero exactly
//! when a build would break.
//!
//! Hermetic by construction: every case runs the child with a fixture PATH
//! (fake `deka`/`dsc` shell scripts echoing fixed versions), a temp cwd, no
//! inherited compiler-resolution overrides, and — via `paired_deka()` — a
//! *copy* of the cli binary with a fixture `dsc` sibling, so the
//! sibling-keyed isolate lookup is under test control. Nothing reads the
//! test runner's own PATH or install.

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

/// A copy of the cli binary in its own directory, with a fixture `dsc`
/// sibling — the isolate compile path resolves dsc sibling-only, so tidy
/// cases need a controlled sibling next to the running binary.
fn paired_deka(root: &Path, dsc_version: Option<&str>) -> (PathBuf, PathBuf) {
    let dir = root.join("running");
    fs::create_dir_all(&dir).unwrap();
    let deka = dir.join("deka");
    fs::copy(cli_bin(), &deka).unwrap();
    let mut perms = fs::metadata(&deka).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&deka, perms).unwrap();
    if let Some(version) = dsc_version {
        executable_script(
            &dir.join("dsc"),
            &format!("#!/bin/sh\necho 'dsc [version {version}]'\n"),
        );
    }
    (deka, dir)
}

struct DoctorRun {
    status: std::process::ExitStatus,
    output: String,
}

/// Run `deka self doctor` in a hermetic child environment: fixture PATH,
/// temp cwd, no inherited compiler-resolution overrides.
fn run_doctor(
    deka_bin: &Path,
    cwd: &Path,
    path_dirs: &[&Path],
    extra_env: &[(&str, &str)],
) -> DoctorRun {
    let path = std::env::join_paths(path_dirs).unwrap();
    let mut command = Command::new(deka_bin);
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
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(&deka, cwd.path(), &[&first.dir, &second.dir], &[]);
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
fn doctor_skips_non_executable_path_entries_when_crowning_the_winner() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let shadow = root.path().join("shadow");
    let real = FixtureBin::new(root.path(), "real", &version, &version);
    fs::create_dir_all(&shadow).unwrap();
    // A non-executable deka first on PATH: the shell would skip it.
    fs::write(shadow.join("deka"), "#!/bin/sh\necho deka\n").unwrap();
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(&deka, cwd.path(), &[&shadow, &real.dir], &[]);
    assert!(run.status.success(), "untidy PATH must not fail: {}", run.output);
    assert!(
        run.output.contains("(not executable, skipped by the shell)"),
        "non-executable entry must be annotated:\n{}",
        run.output
    );
    assert!(
        run
            .output
            .contains(&format!("deka [version {version}]   <- first on PATH, wins")),
        "the executable entry, not the first file, must win:\n{}",
        run.output
    );
}

#[test]
fn doctor_warns_that_a_path_only_dsc_is_invisible_to_isolate_compiles() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    // dsc only on PATH: the CLI path resolves it, the sibling-only isolate
    // path does not — doctor must not call this tidy.
    let bin = FixtureBin::new(root.path(), "bin", &version, &version);
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(Path::new(cli_bin()), cwd.path(), &[&bin.dir], &[]);
    assert!(
        !run.status.success(),
        "a dsc the isolate cannot see must exit non-zero: {}",
        run.output
    );
    assert!(
        run.output.contains(&format!("resolved: {} (via PATH)", bin.dir.join("dsc").display())),
        "cli pairing must name the PATH winner:\n{}",
        run.output
    );
    assert!(
        run.output.contains("isolate (dev/serve/run compiles; sibling-only"),
        "isolate pairing section missing:\n{}",
        run.output
    );
    assert!(
        run.output.contains("resolved: none (not found)"),
        "isolate pairing must report no dsc:\n{}",
        run.output
    );
    assert!(
        run.output.contains("no dsc resolvable for isolate"),
        "isolate gap finding missing:\n{}",
        run.output
    );
}

#[test]
fn doctor_fails_on_deka_dsc_version_mismatch() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    // A dsc version that never equals the running binary's.
    let skewed = "0.0.1";
    let override_dir = FixtureBin::new(root.path(), "override", &version, skewed);
    // Sibling dsc matches, so the mismatch is attributable to DEKA_DSC alone.
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(
        &deka,
        cwd.path(),
        &[],
        &[("DEKA_DSC", override_dir.dir.join("dsc").to_str().unwrap())],
    );
    assert!(!run.status.success(), "mismatch must exit non-zero: {}", run.output);
    assert!(
        run.output.contains("ERROR") && run.output.contains("mismatch"),
        "mismatch finding missing:\n{}",
        run.output
    );
    assert!(
        run.output.contains("cli (check/fmt/transpile/build)"),
        "the mismatch must be attributed to the cli path:\n{}",
        run.output
    );
}

#[test]
fn doctor_prefers_deka_dsc_when_a_sibling_exists() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let override_dir = FixtureBin::new(root.path(), "override", &version, &version);
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let cwd = tempfile::tempdir().unwrap();
    let override_dsc = override_dir.dir.join("dsc");

    let run = run_doctor(
        &deka,
        cwd.path(),
        &[],
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
}

#[test]
fn doctor_reports_isolate_gap_when_dsc_is_only_reachable_via_deka_dsc() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let override_dir = FixtureBin::new(root.path(), "override", &version, &version);
    // No sibling beside the real binary: CLI resolves via DEKA_DSC, the
    // isolate path resolves nothing.
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(
        Path::new(cli_bin()),
        cwd.path(),
        &[],
        &[("DEKA_DSC", override_dir.dir.join("dsc").to_str().unwrap())],
    );
    assert!(
        !run.status.success(),
        "cli-tidy but isolate-broken must exit non-zero: {}",
        run.output
    );
    assert!(
        run.output.contains("(via DEKA_DSC)"),
        "cli pairing must report DEKA_DSC:\n{}",
        run.output
    );
    assert!(
        run.output.contains("no dsc resolvable for isolate"),
        "isolate gap finding missing:\n{}",
        run.output
    );
}

#[test]
fn doctor_rejects_a_deka_dsc_that_is_not_a_file() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("nope/dsc");
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(Path::new(cli_bin()), cwd.path(), &[], &[("DEKA_DSC", missing.to_str().unwrap())]);
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

    let run = run_doctor(Path::new(cli_bin()), cwd.path(), &[&empty.dir], &[]);
    assert!(!run.status.success(), "no dsc must exit non-zero: {}", run.output);
    let cli_finding = run.output.matches("no dsc resolvable for cli").count();
    assert_eq!(
        cli_finding, 1,
        "exactly one cli-path finding expected:\n{}",
        run.output
    );
    assert!(
        run.output.contains("no dsc resolvable for isolate"),
        "isolate-path finding missing:\n{}",
        run.output
    );
}

#[test]
fn doctor_reports_deka_no_dsc_as_a_disabled_warning_only() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let bin = FixtureBin::new(root.path(), "bin", &version, &version);
    // A sibling keeps the isolate path (which ignores DEKA_NO_DSC) resolvable.
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(&deka, cwd.path(), &[&bin.dir], &[("DEKA_NO_DSC", "1")]);
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
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let project = tempfile::tempdir().unwrap();
    write_npm_pin(project.path(), "0.99.0");

    let run = run_doctor(&deka, project.path(), &[], &[]);
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
fn doctor_fails_on_deka_json_pin_skew() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("deka.json"),
        r#"{"name":"app","deka":"0.0.1"}"#,
    )
    .unwrap();

    let run = run_doctor(&deka, project.path(), &[], &[]);
    assert!(!run.status.success(), "deka.json pin skew must exit non-zero: {}", run.output);
    assert!(
        run.output.contains("deka.json \"deka\": 0.0.1"),
        "deka.json pin must be reported:\n{}",
        run.output
    );
    assert!(
        run.output.contains("project pin is older"),
        "must say the project pin is older:\n{}",
        run.output
    );
}

#[test]
fn doctor_passes_when_project_pin_matches_running_binary() {
    let root = tempfile::tempdir().unwrap();
    let version = running_version();
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let project = tempfile::tempdir().unwrap();
    write_npm_pin(project.path(), &version);

    let run = run_doctor(&deka, project.path(), &[], &[]);
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
    let (deka, _dir) = paired_deka(root.path(), Some(&version));
    let cwd = tempfile::tempdir().unwrap();

    let run = run_doctor(&deka, cwd.path(), &[], &[]);
    assert!(run.status.success(), "no project must exit 0: {}", run.output);
    assert!(
        run.output.contains("no project pin found"),
        "no-project note missing:\n{}",
        run.output
    );
}
