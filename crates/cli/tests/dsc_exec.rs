use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

#[test]
fn missing_deka_dsc_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("ok.ds");
    fs::write(&source, "export const answer = 42\n").unwrap();
    let output = Command::new(cli_bin())
        .args(["check", source.to_str().unwrap()])
        .env("DEKA_DSC", "/no/such/dsc-binary")
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("DEKA_DSC"), "unexpected stderr: {stderr}");
}

#[test]
fn missing_dsc_without_fallback_is_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("ok.ds");
    fs::write(&source, "export const answer = 42\n").unwrap();
    // Discovery includes a repository-pinned compiler as well as sibling and
    // PATH binaries. Disable discovery explicitly instead of depending on the
    // host installation, and pin the no-fallback contract for the registered compiler exec commands.
    for command in ["check", "fmt", "transpile"] {
        let output = Command::new(cli_bin())
            .args([command, source.to_str().unwrap()])
            .env_remove("DEKA_DSC")
            .env("DEKA_NO_DSC", "1")
            .env("PATH", "/usr/bin:/bin")
            .output()
            .expect("run deka");
        assert!(!output.status.success(), "{command} must require dsc");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("dsc is required"), "{command}: {stderr}");
    }
}

#[test]
fn deka_no_dsc_does_not_fall_back_in_process() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("ok.ds");
    fs::write(&source, "export const answer = 42\n").unwrap();
    let output = Command::new(cli_bin())
        .args(["check", source.to_str().unwrap()])
        .env("DEKA_NO_DSC", "1")
        .env_remove("DEKA_DSC")
        .output()
        .expect("run deka");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("dsc is required"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn deka_dsc_execs_the_configured_binary() {
    let temp = tempfile::tempdir().unwrap();
    let stub = temp.path().join("dsc");
    fs::write(&stub, "#!/bin/sh\necho DSC_STUB \"$@\"\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();

    let output = Command::new(cli_bin())
        .args(["check", "ignored.ds"])
        .env("DEKA_DSC", stub.to_str().unwrap())
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("DSC_STUB") && stdout.contains("check"),
        "did not exec stub: {stdout}"
    );
}

#[test]
fn failing_check_names_the_producing_binaries() {
    let temp = tempfile::tempdir().unwrap();
    // A dsc stand-in that reports a version and fails like a parse error.
    // deka#1101: the failure output must name the dsc and deka binaries that
    // produced the diagnostic, not only the user's source file.
    let stub = temp.path().join("dsc");
    fs::write(
        &stub,
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then echo 'dsc 0.99.0-test'; exit 0; fi\n\
         echo '[check] 3:1: bad.ds: expected ``)``, found ``}``' >&2\n\
         exit 1\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();

    let bad = temp.path().join("bad.ds");
    fs::write(&bad, "export fn main() {\n").unwrap();

    let output = Command::new(cli_bin())
        .args(["check", bad.to_str().unwrap()])
        .env("DEKA_DSC", &stub)
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("expected ``)``, found ``}``"),
        "dsc's own diagnostic must be preserved: {stderr}"
    );
    assert!(
        stderr.contains("dsc 0.99.0-test"),
        "expected dsc version: {stderr}"
    );
    assert!(
        stderr.contains(stub.to_str().unwrap()),
        "expected dsc path: {stderr}"
    );
    assert!(
        stderr.contains(&format!("deka {}", env!("CARGO_PKG_VERSION"))),
        "expected deka version: {stderr}"
    );
    assert!(
        stderr.contains(cli_bin()),
        "expected deka binary path: {stderr}"
    );
}
