// Version-skew detection between the running deka binary and the project's
// npm-installed deka (deka#1101). A stale globally-installed deka shadowing
// the project's own `./node_modules/.bin/deka` used to fail on a newer
// scaffold with a parse error naming only the user's source file; the CLI
// must call out the mismatch before doing work.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn write_project_deka(root: &std::path::Path, version: &str) {
    let dir = root.join("node_modules/@dekaruntime/deka");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("package.json"),
        format!(r#"{{"name":"@dekaruntime/deka","version":"{version}"}}"#),
    )
    .unwrap();
}

fn dsc_stub(root: &std::path::Path) -> std::path::PathBuf {
    let stub = root.join("dsc");
    fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    stub
}

#[test]
fn skewed_project_version_warns_with_both_versions_and_binary_path() {
    let temp = tempfile::tempdir().unwrap();
    write_project_deka(temp.path(), "999.0.0");
    let source = temp.path().join("ok.ds");
    fs::write(&source, "export const answer = 42\n").unwrap();
    let stub = dsc_stub(temp.path());

    let output = Command::new(cli_bin())
        .args(["check", source.to_str().unwrap()])
        .current_dir(temp.path())
        .env("DEKA_DSC", &stub)
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("this project expects deka 999.0.0"),
        "expected version warning: {stderr}"
    );
    assert!(
        stderr.contains(&format!("running deka {}", env!("CARGO_PKG_VERSION"))),
        "expected running version: {stderr}"
    );
    assert!(
        stderr.contains(cli_bin()),
        "expected running binary path: {stderr}"
    );
}

#[test]
fn matching_project_version_stays_silent() {
    let temp = tempfile::tempdir().unwrap();
    write_project_deka(temp.path(), env!("CARGO_PKG_VERSION"));
    let source = temp.path().join("ok.ds");
    fs::write(&source, "export const answer = 42\n").unwrap();
    let stub = dsc_stub(temp.path());

    let output = Command::new(cli_bin())
        .args(["check", source.to_str().unwrap()])
        .current_dir(temp.path())
        .env("DEKA_DSC", &stub)
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("this project expects deka"),
        "no skew, no warning: {stderr}"
    );
}

#[test]
fn project_without_npm_install_stays_silent() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("ok.ds");
    fs::write(&source, "export const answer = 42\n").unwrap();
    let stub = dsc_stub(temp.path());

    let output = Command::new(cli_bin())
        .args(["check", source.to_str().unwrap()])
        .current_dir(temp.path())
        .env("DEKA_DSC", &stub)
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("this project expects deka"),
        "no npm deka, no warning: {stderr}"
    );
}
