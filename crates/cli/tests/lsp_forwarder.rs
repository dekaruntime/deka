use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

#[test]
fn lsp_forwards_to_located_dsc() {
    let temp = tempfile::tempdir().unwrap();
    let stub = temp.path().join("dsc");
    fs::write(&stub, "#!/bin/sh\necho DSC_STUB \"$@\"\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();

    let output = Command::new(cli_bin())
        .args(["lsp", "--stdio"])
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
        stdout.contains("DSC_STUB") && stdout.contains("lsp") && stdout.contains("--stdio"),
        "did not forward argv to dsc: {stdout}"
    );
}
