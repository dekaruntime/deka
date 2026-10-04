use std::{
    fs,
    process::{Command, Output},
};

fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn runtime_stdout(output: Output) -> Vec<u8> {
    let output = success(output);
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn nested_generic_calls_check_run_and_survive_source_free_relocation() {
    let guide = include_str!("../../../docs/dekascript/native/generic-calls.mdx");
    let source = guide
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("main.ds"), source).unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    success(
        Command::new(cli)
            .args(["check", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap(),
    );
    let expected = b"8\n[[9]]\n2\n";
    assert_eq!(
        runtime_stdout(
            Command::new(cli)
                .args(["run", "main.ds", "--entry", "main"])
                .current_dir(project.path())
                .output()
                .unwrap()
        ),
        expected
    );
    let out = tempfile::tempdir().unwrap();
    let executable = out.path().join("generic-app");
    runtime_stdout(
        Command::new(cli)
            .args(["build", "main.ds", "--entry", "main", "--outfile"])
            .arg(&executable)
            .current_dir(project.path())
            .output()
            .unwrap(),
    );
    drop(project);
    let relocated = tempfile::tempdir().unwrap();
    let moved = relocated.path().join("app");
    fs::rename(executable, &moved).unwrap();
    assert_eq!(
        runtime_stdout(
            Command::new(moved)
                .current_dir(relocated.path())
                .env_clear()
                .output()
                .unwrap()
        ),
        expected
    );
}

#[test]
fn explicit_nested_type_arguments_still_reject_the_wrong_value_type() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("main.ds"),
        r#"fn identity<T>(value: T) T { return value; }
fn main() { const value = identity<Array<number>>("wrong"); }"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_deka"))
        .args(["check", "main.ds", "--entry", "main"])
        .current_dir(project.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("Array<number>"), "{diagnostic}");
    assert!(diagnostic.contains("string"), "{diagnostic}");
    assert!(!diagnostic.contains("comparison"), "{diagnostic}");
}
