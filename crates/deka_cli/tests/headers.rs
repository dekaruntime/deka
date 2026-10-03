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
fn native_headers_check_run_and_source_free_build_share_the_catalog() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("main.ds");
    std::fs::write(
        &source,
        r#"import {echo} from "io";
fn main() {
    const h=unwrap(Headers([["X","one"],["x","two"]])) or{echo("constructor");return;};
    const shared=h;
    const changed=unwrap(shared.append("X","three")) or{echo("append");return;};
    const value=unwrap(h.get("x")) or{echo("get");return;};
    echo(match value{Some(text)=>text,None=>"missing"});
    echo(match h.get("bad name"){Ok(value)=>"accepted",Err(message)=>"invalid"});
    const missing=unwrap(h.get("missing")) or{echo("get missing");return;};
    echo(match missing{Some(text)=>text,None=>"missing"});
    echo(JSON.stringify(h.entries()));
}"#,
    )
    .unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    ok(Command::new(cli)
        .args(["check", "main.ds", "--entry", "main"])
        .current_dir(project.path())
        .output()
        .unwrap());
    let expected = b"one, two, three\ninvalid\nmissing\n[[\"x\",\"one, two, three\"]]\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        expected
    );
    let output = tempfile::tempdir().unwrap();
    let binary = output.path().join("headers-app");
    ok(Command::new(cli)
        .args(["build", "main.ds", "--entry", "main", "--outfile"])
        .arg(&binary)
        .current_dir(project.path())
        .output()
        .unwrap());
    drop(project);
    let elsewhere = tempfile::tempdir().unwrap();
    let moved = elsewhere.path().join("app");
    std::fs::rename(binary, &moved).unwrap();
    assert_eq!(
        ok(Command::new(&moved)
            .current_dir(elsewhere.path())
            .env_clear()
            .output()
            .unwrap()),
        expected
    );
}
