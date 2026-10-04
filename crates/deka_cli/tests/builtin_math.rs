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
fn math_guide_checks_runs_builds_and_survives_source_deletion() {
    let guide = include_str!("../../../docs/dekascript/native/math.mdx");
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
    ok(Command::new(cli)
        .args(["check", "main.ds", "--entry", "main"])
        .current_dir(project.path())
        .output()
        .unwrap());
    let expected = b"3.141592653589793\n3\nSome(4)\nNone\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        expected
    );
    let out = tempfile::tempdir().unwrap();
    let binary = out.path().join("math-app");
    ok(Command::new(cli)
        .args(["build", "main.ds", "--entry", "main", "--outfile"])
        .arg(&binary)
        .current_dir(project.path())
        .output()
        .unwrap());
    drop(project);
    let destination = tempfile::tempdir().unwrap();
    let moved = destination.path().join("app");
    std::fs::rename(binary, &moved).unwrap();
    assert_eq!(
        ok(Command::new(moved)
            .current_dir(destination.path())
            .env_clear()
            .output()
            .unwrap()),
        expected
    );
}
