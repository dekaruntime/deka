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
fn tcp_documentation_runs_from_source_and_relocated_source_free_binary() {
    let source = include_str!("../../../docs/dekascript/native/tcp.mdx")
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("main.ds"), source).unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    check::checked(
        Command::new(cli)
            .args(["check", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap(),
        "main.ds",
    );
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        b"Deka\n"
    );
    let dest = tempfile::tempdir().unwrap();
    let bin = dest.path().join("tcp-app");
    ok(Command::new(cli)
        .args(["build", "main.ds", "--entry", "main", "--outfile"])
        .arg(&bin)
        .current_dir(project.path())
        .output()
        .unwrap());
    drop(project);
    let moved = tempfile::tempdir().unwrap();
    let path = moved.path().join("app");
    std::fs::rename(bin, &path).unwrap();
    assert_eq!(
        ok(Command::new(path)
            .env_clear()
            .current_dir(moved.path())
            .output()
            .unwrap()),
        b"Deka\n"
    );
}
