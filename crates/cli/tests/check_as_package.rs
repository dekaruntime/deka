use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn run_check_as_package(package: &Path) -> std::process::Output {
    Command::new(cli_bin())
        .args(["check", "--as-package", package.to_str().unwrap()])
        .output()
        .expect("run deka check --as-package")
}

fn write_package(dir: &Path, entry: &str) {
    fs::write(
        dir.join("deka.json"),
        r#"{"name":"@deka/local-check","version":"0.1.0","main":"index.ds"}
"#,
    )
    .unwrap();
    fs::write(dir.join("index.ds"), entry).unwrap();
}

#[test]
fn sound_package_passes() {
    let package = tempfile::tempdir().unwrap();
    write_package(
        package.path(),
        "export fn greet(name: string) string {\n  return name\n}\n",
    );

    let output = run_check_as_package(package.path());
    assert!(
        output.status.success(),
        "sound package failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn type_error_in_package_body_fails_with_diagnostic() {
    let package = tempfile::tempdir().unwrap();
    write_package(
        package.path(),
        "export fn greet(name: string) string {\n  return 42\n}\n",
    );

    let output = run_check_as_package(package.path());
    assert!(
        !output.status.success(),
        "broken package passed the gate: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("expected return type `string`, found type `number`"),
        "diagnostic missing from output: {stderr}"
    );
}

#[test]
fn error_inside_a_non_entry_module_fails() {
    // The broken code sits in a module the entry re-exports through, so the
    // gate only catches it if the whole graph reachable from the consumer is
    // typechecked, not just the entry file.
    let package = tempfile::tempdir().unwrap();
    fs::write(
        package.path().join("deka.json"),
        r#"{"name":"@deka/local-check","version":"0.1.0","main":"index.ds"}
"#,
    )
    .unwrap();
    fs::write(
        package.path().join("index.ds"),
        "import { helper } from \"./helper.ds\"\nexport { helper }\n",
    )
    .unwrap();
    fs::write(
        package.path().join("helper.ds"),
        "export fn helper(name: string) string {\n  return 42\n}\n",
    )
    .unwrap();

    let output = run_check_as_package(package.path());
    assert!(
        !output.status.success(),
        "broken dependency module passed the gate: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn type_only_exports_are_checked() {
    let package = tempfile::tempdir().unwrap();
    write_package(
        package.path(),
        "struct User {\n  name: string;\n}\n\nfn make(name: string) User {\n  return User { name: name }\n}\n\nexport { User, make }\n",
    );

    let output = run_check_as_package(package.path());
    assert!(
        output.status.success(),
        "package with type exports failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn package_with_no_exports_is_rejected() {
    let package = tempfile::tempdir().unwrap();
    write_package(package.path(), "fn internal(): number {\n  return 1\n}\n");

    let output = run_check_as_package(package.path());
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("exports nothing"),
        "unexpected output: {stderr}"
    );
}

#[test]
fn missing_manifest_is_rejected() {
    let package = tempfile::tempdir().unwrap();
    let output = run_check_as_package(package.path());
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot read package deka.json"),
        "unexpected output: {stderr}"
    );
}
