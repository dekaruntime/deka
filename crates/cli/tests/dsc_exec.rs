use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

#[test]
fn missing_deka_dsc_fails_closed() {
    let output = Command::new(cli_bin())
        .args(["check", "--help"])
        .env("DEKA_DSC", "/no/such/dsc-binary")
        .env_remove("DEKA_NO_DSC")
        .output()
        .expect("run deka");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("DEKA_DSC"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn deka_no_dsc_uses_in_process_compiler() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("ok.ds");
    fs::write(&source, "export const answer = 42\n").unwrap();
    let output = Command::new(cli_bin())
        .args(["check", source.to_str().unwrap()])
        .env("DEKA_NO_DSC", "1")
        .env_remove("DEKA_DSC")
        .output()
        .expect("run deka");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn deka_dsc_execs_the_configured_binary() {
    let temp = tempfile::tempdir().unwrap();
    let stub = temp.path().join("dsc");
    fs::write(
        &stub,
        "#!/bin/sh\necho DSC_STUB \"$@\"\nexit 0\n",
    )
    .unwrap();
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
