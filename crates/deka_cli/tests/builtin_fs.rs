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
fn filesystem_guide_has_real_effects_and_survives_source_deletion() {
    let guide = include_str!("../../../docs/dekascript/native/fs.mdx");
    let example = guide
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("main.ds"), example).unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    check::checked(
        Command::new(cli)
            .args(["check", "main.ds", "--entry", "main"])
            .current_dir(source.path())
            .output()
            .unwrap(),
        "main.ds",
    );
    let expected = b"true\n6\nnative\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(source.path())
            .output()
            .unwrap()),
        expected
    );
    assert_eq!(
        std::fs::read(source.path().join("data/note.txt")).unwrap(),
        b"native"
    );
    let output = tempfile::tempdir().unwrap();
    let binary = output.path().join("fs-app");
    ok(Command::new(cli)
        .args(["build", "main.ds", "--entry", "main", "--outfile"])
        .arg(&binary)
        .current_dir(source.path())
        .output()
        .unwrap());
    drop(source);
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
    assert_eq!(
        std::fs::read(relocated.path().join("data/note.txt")).unwrap(),
        b"native"
    );
}
