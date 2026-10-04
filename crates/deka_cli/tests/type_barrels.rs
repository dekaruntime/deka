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
fn type_barrel_guide_checks_as_package_runs_and_survives_source_deletion() {
    let guide = include_str!("../../../docs/dekascript/native/type-barrels.mdx");
    let project = tempfile::tempdir().unwrap();
    for (name, chunk) in ["library.ds", "index.ds", "main.ds"]
        .into_iter()
        .zip(guide.split("```ds\n").skip(1))
    {
        std::fs::write(
            project.path().join(name),
            chunk.split("```").next().unwrap(),
        )
        .unwrap();
    }
    std::fs::write(
        project.path().join("deka.json"),
        r#"{"name":"@deka/type-barrel","version":"9.1.2","entry":"index.ds"}"#,
    )
    .unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    check::checked(
        Command::new(cli)
            .args(["check", "--as-package", "."])
            .current_dir(project.path())
            .output()
            .unwrap(),
        ".",
    );
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
        b"42\n"
    );
    let out = tempfile::tempdir().unwrap();
    let binary = out.path().join("type-app");
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
        b"42\n"
    );
}
