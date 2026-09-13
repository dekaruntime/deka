//! The same real-compiler smoke runs on every native CI host.
#![cfg(feature = "native")]
use std::{fs, process::Command};

#[test]
fn compiled_program_runs_without_sources_or_compiler_from_another_directory() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(
        source.path().join("hello.ds"),
        "import { echo } from \"io\"\necho(\"hello from compiled Deka\");\n",
    )
    .unwrap();
    // Reuse the checked-in @deka/io 0.1.1 package, including its real host
    // implementation and lockfile; no compiler or runtime stubs.
    let io = source.path().join("ds_modules/@deka/io");
    fs::create_dir_all(&io).unwrap();
    fs::write(
        io.join("index.ds"),
        include_str!("fixtures/package-cache/cache-entry/ds_modules/@deka/io/index.ds"),
    )
    .unwrap();
    fs::write(
        io.join("deka.json"),
        include_str!("fixtures/package-cache/cache-entry/ds_modules/@deka/io/deka.json"),
    )
    .unwrap();
    fs::write(
        source.path().join("deka.lock"),
        include_str!("fixtures/package-cache/cache-entry/deka.lock"),
    )
    .unwrap();
    fs::write(
        source.path().join("deka.json"),
        r#"{"dependencies":{"@deka/io":"0.1.1"}}"#,
    )
    .unwrap();
    // A decoy proves the positional entry is honored.
    fs::write(source.path().join("app.ds"), "invalid source").unwrap();
    let dsc = compiler::dsc::find_cli_dsc()
        .unwrap()
        .expect("smoke requires real dsc; set DEKA_DSC");
    let private_dsc = source.path().join("dsc");
    fs::copy(dsc, &private_dsc).unwrap();
    let executable = destination.path().join("hello");
    let output = Command::new(env!("CARGO_BIN_EXE_cli"))
        .current_dir(source.path())
        .env("DEKA_DSC", &private_dsc)
        .env_remove("DEKA_NO_DSC")
        .args(["compile", "hello.ds", "--outfile"])
        .arg(&executable)
        .output()
        .unwrap();
    assert!(output.status.success(), "compile: {output:?}");
    source.close().unwrap();
    assert!(
        !private_dsc.exists(),
        "the selected compiler must be removed"
    );
    #[cfg(target_os = "macos")]
    {
        let signature = Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(&executable)
            .output()
            .unwrap();
        assert!(signature.status.success(), "signature: {signature:?}");
    }
    let output = Command::new(&executable)
        .current_dir(elsewhere.path())
        .env_clear()
        .env("PATH", elsewhere.path())
        .env("HOME", elsewhere.path())
        .env("DEKA_DSC", elsewhere.path().join("missing-dsc"))
        .env("DEKA_NO_DSC", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "execution: {output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "hello from compiled Deka\n",
        "{output:?}"
    );
}

#[test]
fn compile_rejects_invalid_input_and_preserves_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bad.ds"), "this is not DekaScript").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cli"))
        .current_dir(dir.path())
        .args(["compile", "bad.ds"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(!dir.path().join("deka-app").exists());
    fs::write(dir.path().join("deka-app"), "keep me").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cli"))
        .current_dir(dir.path())
        .args(["compile", "bad.ds"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("deka-app")).unwrap(),
        "keep me"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_cli"))
        .current_dir(dir.path())
        .args(["compile", "bad.ds", "--target", "linux-x64"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "cross-target selection must be rejected: {output:?}"
    );
}
