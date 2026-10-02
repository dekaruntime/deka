use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn cli(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deka"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}
fn ok(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn source_modules_tests_and_relocated_single_file_work_without_dsc() {
    let project = tempfile::tempdir().unwrap();
    let directory = project.path();
    fs::write(
        directory.join("math.ds"),
        "export const base = 40; export fn answer() number { return base + 2; }",
    )
    .unwrap();
    fs::write(directory.join("main.ds"), "import { echo } from \"io\"; import { assert } from \"test\"; import { answer } from \"./math.ds\"; assert(answer() == 42); echo(\"42\");").unwrap();
    assert_eq!(ok(cli(directory, &["run", "main.ds"])), "42\n");
    fs::write(directory.join("math.test.ds"), "import { assert } from \"test\"; import { answer } from \"./math.ds\"; fn test_answer() { assert(answer() == 42); } fn test_independent() { assert(2 + 2 == 4); }").unwrap();
    let result = ok(cli(directory, &["test"]));
    assert!(result.contains("2 passed, 0 failed"), "{result}");
    let out = tempfile::tempdir().unwrap();
    let executable = out.path().join("answer");
    ok(cli(
        directory,
        &[
            "build",
            "main.ds",
            "--outfile",
            executable.to_str().unwrap(),
        ],
    ));
    drop(project);
    let elsewhere = tempfile::tempdir().unwrap();
    let output = Command::new(&executable)
        .current_dir(elsewhere.path())
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(ok(output), "42\n");
    #[cfg(target_os = "macos")]
    assert!(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(executable)
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn failing_assertion_and_empty_discovery_exit_unsuccessfully() {
    let project = tempfile::tempdir().unwrap();
    assert!(!cli(project.path(), &["test"]).status.success());
    fs::write(project.path().join("broken.test.ds"), "import { assert } from \"test\"; fn test_fail() { assert(false); } fn test_pass() { assert(true); }").unwrap();
    let output = cli(project.path(), &["test"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed, 1 failed"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("assertion failed"));
}
#[test]
fn scaffolded_app_builds_and_clicks_after_source_is_removed() {
    let project = tempfile::tempdir().unwrap();
    ok(cli(project.path(), &["init"]));
    assert!(ok(cli(project.path(), &["test"])).contains("1 passed, 0 failed"));
    assert!(ok(cli(project.path(), &["run", "--exercise", "3"])).contains("Count:  3"));
    let out = tempfile::tempdir().unwrap();
    let executable = out.path().join("app");
    ok(cli(
        project.path(),
        &["build", "--outfile", executable.to_str().unwrap()],
    ));
    drop(project);
    let output = Command::new(executable)
        .current_dir(out.path())
        .args(["--exercise", "4"])
        .env_clear()
        .output()
        .unwrap();
    assert!(ok(output).contains("Count:  4"));
}
#[test]
fn invalid_source_preserves_output_and_self_import_cycles_fail() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    fs::write(p.join("bad.ds"), "this is invalid").unwrap();
    fs::write(p.join("existing"), b"keep this").unwrap();
    assert!(
        !cli(p, &["build", "bad.ds", "--outfile", "existing"])
            .status
            .success()
    );
    assert_eq!(fs::read(p.join("existing")).unwrap(), b"keep this");
    fs::write(
        p.join("selfish.ds"),
        "import { s } from \"./selfish.ds\"; export fn s() number { return 1; }",
    )
    .unwrap();
    let result = cli(p, &["check", "selfish.ds"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("cyclic module import"));
    // Import cycles between files load with JavaScript module semantics:
    // run-time calls across the cycle work once both files have loaded.
    fs::write(
        p.join("a.ds"),
        "import { b } from \"./b.ds\"; export fn a() number { return 1; } fn main() number { return a() + b(); }",
    )
    .unwrap();
    fs::write(
        p.join("b.ds"),
        "import { a } from \"./a.ds\"; export fn b() number { return a() + 1; }",
    )
    .unwrap();
    assert_eq!(ok(cli(p, &["run", "a.ds", "--entry", "main"])), "");
}

#[test]
fn init_preserves_npm_files_and_never_overwrites_source() {
    let project = tempfile::tempdir().unwrap();
    let path = project.path();
    fs::write(path.join("package.json"), "{\"private\":true}").unwrap();
    ok(cli(path, &["init"]));
    assert_eq!(
        fs::read_to_string(path.join("package.json")).unwrap(),
        "{\"private\":true}"
    );
    let source = fs::read(path.join("App.dsx")).unwrap();
    assert!(!cli(path, &["init"]).status.success());
    assert_eq!(fs::read(path.join("App.dsx")).unwrap(), source);
}

#[test]
fn separate_modules_keep_private_bindings_and_run_shared_initializers_once() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    fs::write(
        p.join("shared.ds"),
        "import { echo } from \"io\"; echo(\"initialized\"); export const value = 1;",
    )
    .unwrap();
    fs::write(p.join("a.ds"), "import { value } from \"./shared.ds\"; const private = 10; export fn a() number { return private + value; }").unwrap();
    fs::write(p.join("b.ds"), "import { value } from \"./shared.ds\"; const private = 20; export fn b() number { return private + value; }").unwrap();
    fs::write(p.join("main.ds"), "import { assert } from \"test\"; import { a } from \"./a.ds\"; import { b } from \"./b.ds\"; assert(a() == 11); assert(b() == 21);").unwrap();
    assert_eq!(ok(cli(p, &["run", "main.ds"])), "initialized\n");
}

#[test]
fn an_async_export_in_a_dependency_does_not_change_the_root_entry() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    fs::write(
        p.join("helper.ds"),
        "export async fn main() Promise<number> { return 1; } export const value = 42;",
    )
    .unwrap();
    fs::write(p.join("main.ds"), "import { assert } from \"test\"; import { value } from \"./helper.ds\"; fn main() { assert(value == 42); }").unwrap();
    ok(cli(p, &["run", "main.ds", "--entry", "main"]));
    fs::write(p.join("async.test.ds"), "import { assert } from \"test\"; async fn value() Promise<number> { return 42; } async fn test_value() Promise<void> { assert(await value() == 42); }").unwrap();
    assert!(ok(cli(p, &["test"])).contains("1 passed, 0 failed"));
}
