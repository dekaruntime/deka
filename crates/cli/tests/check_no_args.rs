//! deka#1100: bare `deka check` (no file argument) used to exec straight
//! through to dsc, so a new user's first encounter was the inner tool's own
//! usage text (`[check] usage: dsc check <file.ds>`). Bare `deka check` must
//! speak in deka's voice: check the project when `deka.json` is present,
//! print deka's own usage line when it is not.
//!
//! Everything runs against a stub `dsc` (via the pre-existing DEKA_DSC test
//! hook) so no real toolchain is needed. The stub mimics the released
//! compiler's argv handling for the pre-fix exec-through path — including
//! printing dsc's usage when invoked without a file — so these tests fail
//! against the unfixed CLI for exactly the reported reason.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const DSC_USAGE: &str = "[check] usage: dsc check <file.ds>";
const DEKA_USAGE: &str = "usage: deka check <file.ds>";

/// Stub `dsc`: `check <file>` fails (like the real compiler) when the file
/// contains the BROKEN marker, succeeds otherwise, and records every check
/// invocation. Invoked with no file — the pre-fix outcome of bare
/// `deka check` — it prints dsc's own usage, matching the released binary.
fn write_dsc_stub(root: &Path) -> PathBuf {
    let stub = root.join("dsc-stub.sh");
    let log = root.join("dsc-check.log");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "dsc stub 1.0.0"
  exit 0
fi
file=""
for arg in "$@"; do
  case "$arg" in
    --*) ;;
    check) ;;
    *) [ -z "$file" ] && file="$arg" ;;
  esac
done
if [ -z "$file" ]; then
  echo "{usage}" >&2
  exit 1
fi
echo "$file" >> "{log}"
if grep -q BROKEN "$file"; then
  echo "[check] $file: expected identifier, found BROKEN" >&2
  exit 1
fi
echo "[ok] checked $file"
"#,
        usage = DSC_USAGE,
        log = log.display()
    );
    fs::write(&stub, script).expect("write dsc stub");
    make_executable(&stub);
    stub
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).expect("stat stub").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod stub");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

fn run_bare_check(dir: &Path, dsc: &Path) -> std::process::Output {
    Command::new(cli_bin())
        .arg("check")
        .current_dir(dir)
        .env("DEKA_DSC", dsc)
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka check")
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn check_log(tooling: &Path) -> Vec<String> {
    fs::read_to_string(tooling.join("dsc-check.log"))
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

#[test]
fn bare_check_outside_a_project_prints_deka_usage_not_dsc_usage() {
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let dir = tempfile::tempdir().expect("scratch dir");

    let output = run_bare_check(dir.path(), &dsc);
    let text = combined(&output);

    assert!(
        text.contains(DEKA_USAGE),
        "bare deka check must print deka's own usage: {text}"
    );
    assert!(
        !text.contains(DSC_USAGE) && !text.contains("dsc check"),
        "inner tool usage must never leak: {text}"
    );
    assert_eq!(output.status.code(), Some(2), "usage error exit: {text}");
    assert!(
        check_log(tooling.path()).is_empty(),
        "no project and no file means dsc must not run at all: {text}"
    );
}

#[test]
fn bare_check_in_a_project_checks_every_project_source() {
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let project = tempfile::tempdir().expect("project dir");
    fs::write(project.path().join("deka.json"), "{}\n").expect("write manifest");
    fs::create_dir_all(project.path().join("app")).expect("create app tree");
    fs::create_dir_all(project.path().join("src")).expect("create src tree");
    fs::write(
        project.path().join("app").join("page.ds"),
        "export const ok = true\n",
    )
    .expect("write app source");
    fs::write(
        project.path().join("src").join("lib.ds"),
        "export const ok = true\n",
    )
    .expect("write src source");

    let output = run_bare_check(project.path(), &dsc);
    let text = combined(&output);

    assert!(
        output.status.success(),
        "clean project must pass a bare check: {text}"
    );
    assert!(
        !text.contains(DSC_USAGE),
        "inner tool usage must never leak: {text}"
    );
    assert_eq!(
        check_log(tooling.path()),
        vec!["app/page.ds".to_string(), "src/lib.ds".to_string()],
        "bare deka check must run dsc check on each project source: {text}"
    );
}

#[test]
fn bare_check_in_a_project_fails_on_a_broken_source() {
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let project = tempfile::tempdir().expect("project dir");
    fs::write(project.path().join("deka.json"), "{}\n").expect("write manifest");
    fs::create_dir_all(project.path().join("app")).expect("create app tree");
    fs::write(project.path().join("app").join("page.ds"), "BROKEN\n").expect("write broken source");

    let output = run_bare_check(project.path(), &dsc);
    let text = combined(&output);

    assert!(
        !output.status.success(),
        "broken project source must fail the bare check: {text}"
    );
    assert!(
        text.contains("app/page.ds"),
        "the diagnostic must name the failing source: {text}"
    );
    assert!(
        !text.contains(DSC_USAGE),
        "inner tool usage must never leak: {text}"
    );
}

#[test]
fn check_with_a_file_argument_still_execs_dsc() {
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let dir = tempfile::tempdir().expect("scratch dir");
    let source = dir.path().join("main.ds");
    fs::write(&source, "export const ok = true\n").expect("write source");

    let output = Command::new(cli_bin())
        .args(["check", "main.ds"])
        .current_dir(dir.path())
        .env("DEKA_DSC", &dsc)
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka check main.ds");
    let text = combined(&output);

    assert!(output.status.success(), "file check failed: {text}");
    assert!(
        text.contains("[ok] checked main.ds"),
        "dsc's per-file result must pass through unchanged: {text}"
    );
}
