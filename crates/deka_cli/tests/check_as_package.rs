use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn write(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn package(dir: &Path, source: &str) {
    write(
        dir,
        "deka.json",
        r#"{"name":"@deka/local-check","version":"91.2.3","entry":"index.ds"}"#,
    );
    write(dir, "index.ds", source);
}
fn check(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deka"))
        .current_dir(dir)
        .arg("check")
        .arg("--as-package")
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output, path: &str) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("[check] {path} - ok\n")
    );
}
fn refused(output: Output, text: &str) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains(text), "{error}");
    assert!(error.starts_with("[check] "), "{error}");
}

#[test]
fn unpublished_local_package_checks_without_execution_or_source_mutation() {
    let dir = tempfile::tempdir().unwrap();
    package(
        dir.path(),
        &format!(
            "import {{echo}} from \"io\"; echo(\"must not run\");\n{}",
            include_str!("../../../docs/dekascript/native/package-checking.mdx")
                .split("```ds\n")
                .nth(1)
                .unwrap()
                .split("```")
                .next()
                .unwrap()
        ),
    );
    let before_manifest = fs::read(dir.path().join("deka.json")).unwrap();
    let before_source = fs::read(dir.path().join("index.ds")).unwrap();
    success(check(dir.path(), &[]), ".");
    assert_eq!(
        fs::read(dir.path().join("deka.json")).unwrap(),
        before_manifest
    );
    assert_eq!(
        fs::read(dir.path().join("index.ds")).unwrap(),
        before_source
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    let unrelated = tempfile::tempdir().unwrap();
    success(
        check(unrelated.path(), &[dir.path().to_str().unwrap()]),
        dir.path().to_str().unwrap(),
    );
}

#[test]
fn default_barrel_and_generic_type_exports_are_checked_as_consumer_imports() {
    let dir = tempfile::tempdir().unwrap();
    package(
        dir.path(),
        r#"export {greet as default, answer, Box} from "./surface.ds";
enum Choice { First, Second }
alias Count = number;
interface Named { fn name() string; }
export {Choice, Count, Named};"#,
    );
    write(
        dir.path(),
        "surface.ds",
        r#"
export fn greet() string { return "Hello"; }
export const answer = 42;
struct Box<T> { value: T; }
export {Box};
"#,
    );
    success(check(dir.path(), &["."]), ".");
}

#[test]
fn type_only_and_default_only_packages_have_a_consumer_surface() {
    let dir = tempfile::tempdir().unwrap();
    package(dir.path(), "alias Count = number; export {Count};");
    success(check(dir.path(), &[]), ".");
    write(
        dir.path(),
        "index.ds",
        "export default fn greet() number { return 7; }",
    );
    success(check(dir.path(), &[]), ".");
    write(
        dir.path(),
        "index.ds",
        "alias Count = number; export {Count as default};",
    );
    success(check(dir.path(), &[]), ".");
}

#[test]
fn bad_imported_body_reports_original_path_and_type_cause() {
    let dir = tempfile::tempdir().unwrap();
    package(dir.path(), "export {greet} from \"./helper.ds\";");
    write(
        dir.path(),
        "helper.ds",
        "export fn greet() string { return 42; }",
    );
    let output = check(dir.path(), &[]);
    let error = String::from_utf8_lossy(&output.stderr).to_string();
    refused(output, "expected return type `string`, found type `number`");
    assert!(
        error.contains(&dir.path().join("helper.ds").display().to_string()),
        "{error}"
    );
    assert!(!error.contains("consumer.ds"), "{error}");
}

#[test]
fn installed_dependency_is_used_without_registry_access_and_lock_pin_is_enforced() {
    let dir = tempfile::tempdir().unwrap();
    package(
        dir.path(),
        "import {value} from \"@deka/dependency\"; export fn answer() number {return value;}",
    );
    write(
        dir.path(),
        "deka.json",
        r#"{"name":"@deka/local-check","version":"91.2.3","dependencies":{"@deka/dependency":"1.0.0"}}"#,
    );
    write(
        dir.path(),
        "ds_modules/@deka/dependency/deka.json",
        r#"{"name":"@deka/dependency","version":"1.0.0","dependencies":{}}"#,
    );
    write(
        dir.path(),
        "ds_modules/@deka/dependency/index.ds",
        "export const value = 42;",
    );
    let lock =
        r#"{"lockfileVersion":1,"packages":{"@deka/dependency":["1.0.0","unreachable",{},""]}}"#;
    write(dir.path(), "deka.lock", lock);
    success(check(dir.path(), &[]), ".");
    assert_eq!(
        fs::read_to_string(dir.path().join("deka.lock")).unwrap(),
        lock
    );
    write(dir.path(), "deka.lock", &lock.replace("1.0.0", "2.0.0"));
    refused(check(dir.path(), &[]), "deka.lock pins 2.0.0");
    fs::remove_dir_all(dir.path().join("ds_modules")).unwrap();
    refused(check(dir.path(), &[]), "is not installed");
}

#[test]
fn empty_exports_missing_manifest_and_invalid_entry_are_real_check_failures() {
    let dir = tempfile::tempdir().unwrap();
    refused(check(dir.path(), &[]), "cannot read package deka.json");
    package(dir.path(), "fn internal() number {return 1;}");
    refused(check(dir.path(), &[]), "exports nothing");
    write(
        dir.path(),
        "deka.json",
        r#"{"name":"@deka/local-check","entry":"../outside.ds"}"#,
    );
    refused(check(dir.path(), &[]), "no valid DekaScript entry");
    write(
        dir.path(),
        "deka.json",
        r#"{"name":"@deka/local-check","entry":"missing.ds"}"#,
    );
    refused(check(dir.path(), &[]), "no valid DekaScript entry");
    write(
        dir.path(),
        "deka.json",
        r#"{"name":"math","version":"1.0.0"}"#,
    );
    refused(check(dir.path(), &[]), "resolves to a native builtin");
}

#[test]
fn package_check_rejects_execution_entry_and_extra_paths_as_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    package(dir.path(), "export fn greet() number {return 1;}");
    for args in [&[".", "extra"][..], &[".", "--entry", "greet"][..]] {
        let output = check(dir.path(), args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn consumer_check_catches_a_legacy_dependency_that_only_works_in_the_source_project() {
    let dir = tempfile::tempdir().unwrap();
    package(
        dir.path(),
        "import {answer} from \"@deka/helper\"; export {answer};",
    );
    write(
        dir.path(),
        "deka.json",
        r#"{"name":"@deka/local-check","version":"91.2.3","dependencies":{"@deka/helper":"1.0.0","@deka/private":"1.0.0"}}"#,
    );
    write(
        dir.path(),
        "ds_modules/@deka/helper/deka.json",
        r#"{"name":"@deka/helper","version":"1.0.0"}"#,
    );
    write(
        dir.path(),
        "ds_modules/@deka/helper/index.ds",
        "import {value} from \"@deka/private\"; export fn answer() number {return value;}",
    );
    write(
        dir.path(),
        "ds_modules/@deka/private/deka.json",
        r#"{"name":"@deka/private","version":"1.0.0","dependencies":{}}"#,
    );
    write(
        dir.path(),
        "ds_modules/@deka/private/index.ds",
        "export const value = 42;",
    );
    let producer = Command::new(env!("CARGO_BIN_EXE_deka"))
        .current_dir(dir.path())
        .args(["check", "index.ds"])
        .output()
        .unwrap();
    success(producer, "index.ds");
    refused(
        check(dir.path(), &[]),
        "package @deka/private is not declared in deka.json dependencies",
    );
    write(
        dir.path(),
        "ds_modules/@deka/helper/deka.json",
        r#"{"name":"@deka/helper","version":"1.0.0","dependencies":{"@deka/private":"1.0.0"}}"#,
    );
    success(check(dir.path(), &[]), ".");
}

#[test]
fn package_flag_cannot_silently_run_or_mutate_a_project_through_other_commands() {
    let dir = tempfile::tempdir().unwrap();
    package(
        dir.path(),
        "import {echo} from \"io\"; echo(\"must not run\"); export const value = 1;",
    );
    for args in [
        vec!["run", "index.ds"],
        vec!["index.ds"],
        vec!["build", "index.ds"],
        vec!["install"],
        vec!["test"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_deka"))
            .current_dir(dir.path())
            .args(args)
            .arg("--as-package")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--as-package requires check"));
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
