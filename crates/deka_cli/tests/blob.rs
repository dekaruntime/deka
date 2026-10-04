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
fn blob_file_guide_checks_runs_and_survives_source_deletion_and_relocation() {
    let guide = include_str!("../../../docs/dekascript/native/blob-file.mdx");
    let examples = guide
        .split("```ds\n")
        .skip(1)
        .map(|block| block.split("```").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(examples.len(), 2);
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("main.ds"),format!(r#"import {{echo}} from "io";
    {}
    async fn main(){{echo(await excerpt("abcdef"));echo(await namedContents("notes","notes.txt"));}}"#,examples.join("\n"))).unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    check::checked(
        Command::new(cli)
            .args(["check", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap(),
        "main.ds",
    );
    let expected = b"bcd:text/plain:3\nnotes.txt:notes:notes:5\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        expected
    );
    let out = tempfile::tempdir().unwrap();
    let binary = out.path().join("blob-app");
    ok(Command::new(cli)
        .args(["build", "main.ds", "--entry", "main", "--outfile"])
        .arg(&binary)
        .current_dir(project.path())
        .output()
        .unwrap());
    drop(project);
    let moved_dir = tempfile::tempdir().unwrap();
    let moved = moved_dir.path().join("app");
    std::fs::rename(binary, &moved).unwrap();
    assert_eq!(
        ok(Command::new(moved)
            .current_dir(moved_dir.path())
            .env_clear()
            .output()
            .unwrap()),
        expected
    );
}
