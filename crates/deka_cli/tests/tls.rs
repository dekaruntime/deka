#[path = "support/check.rs"]
mod check;
#[path = "../../deka_vm/tests/support/tls.rs"]
mod identity;
use std::process::{Command, Output};
fn ok(o: Output) -> Vec<u8> {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    o.stdout
}
#[test]
fn tls_documentation_runs_from_source_and_source_free_relocated_binary() {
    let guide = include_str!("../../../docs/dekascript/native/tls.mdx")
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let source = identity::program(guide, &identity::identity());
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
    let destination = tempfile::tempdir().unwrap();
    let executable = destination.path().join("app");
    ok(Command::new(cli)
        .args(["build", "main.ds", "--entry", "main", "--outfile"])
        .arg(&executable)
        .current_dir(project.path())
        .output()
        .unwrap());
    drop(project);
    let relocated = tempfile::tempdir().unwrap();
    let app = relocated.path().join("app");
    std::fs::rename(executable, &app).unwrap();
    assert_eq!(
        ok(Command::new(app)
            .env_clear()
            .current_dir(relocated.path())
            .output()
            .unwrap()),
        b"Deka\n"
    );
}
