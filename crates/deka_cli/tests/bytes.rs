#[path = "support/check.rs"]
mod check;
use std::process::{Command, Output};
fn ok(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
#[test]
fn bytes_guide_checks_runs_builds_and_survives_source_deletion() {
    let guide = include_str!("../../../docs/dekascript/native/bytes.mdx");
    let example = guide
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("main.ds"), example).unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    check::checked(
        Command::new(cli)
            .args(["check", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap(),
        "main.ds",
    );
    let expected = b"68656c6c6f\nlo\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        expected
    );
    let out = tempfile::tempdir().unwrap();
    let binary = out.path().join("bytes-app");
    ok(Command::new(cli)
        .args(["build", "main.ds", "--entry", "main", "--outfile"])
        .arg(&binary)
        .current_dir(project.path())
        .output()
        .unwrap());
    drop(project);
    let relocated = tempfile::tempdir().unwrap();
    let moved = relocated.path().join("app");
    std::fs::rename(binary, &moved).unwrap();
    assert_eq!(
        ok(Command::new(moved)
            .current_dir(relocated.path())
            .env_clear()
            .output()
            .unwrap()),
        expected
    );
}
