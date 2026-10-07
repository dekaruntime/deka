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
fn jwt_guide_checks_runs_and_survives_source_deletion_and_relocation() {
    let guide = include_str!("../../../docs/dekascript/native/jwt-module.mdx");
    let source = guide
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("main.ds"), source).unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    let checked = Command::new(cli)
        .args(["check", "main.ds", "--entry", "main"])
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(checked.stdout.is_empty());
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        b"user-7\n"
    );
    let output = tempfile::tempdir().unwrap();
    let executable = output.path().join("jwt-app");
    ok(Command::new(cli)
        .args(["build", "main.ds", "--entry", "main", "--outfile"])
        .arg(&executable)
        .current_dir(project.path())
        .output()
        .unwrap());
    drop(project);
    let relocated = tempfile::tempdir().unwrap();
    let executable2 = relocated.path().join("app");
    std::fs::rename(executable, &executable2).unwrap();
    assert_eq!(
        ok(Command::new(executable2)
            .current_dir(relocated.path())
            .env_clear()
            .output()
            .unwrap()),
        b"user-7\n"
    );
}
