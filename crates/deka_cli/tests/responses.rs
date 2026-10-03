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
fn native_response_check_run_and_source_free_build_share_typed_body_consumption() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("main.ds");
    std::fs::write(&source,r#"import {echo} from "io";
struct Project { title: string; }
fn (p Project) label() string {return "Project: " + p.title;}
async fn main() {
    const response=unwrap(Response(JSON.stringify(Project{title:"Deka"}))) or {echo("constructor");return;};
    echo(string(response.status));
    echo(string(response.ok));
    echo(string(response.bodyUsed));
    const parsed=response.json<Project>();
    echo(string(response.bodyUsed));
    const project=unwrap(await parsed) or {echo("parse");return;};
    echo(project.label());
    echo(match await response.text(){Ok(t)=>"accepted",Err(e)=>e});
    const text=unwrap(Response("Hello, 雪")) or{echo("constructor");return;};
    const output=unwrap(await text.text()) or{echo("read failed");return;};
    echo(output);
}"#).unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    ok(Command::new(cli)
        .args(["check", "main.ds", "--entry", "main"])
        .current_dir(project.path())
        .output()
        .unwrap());
    let expected=b"200\ntrue\nfalse\ntrue\nProject: Deka\nresponse body has already been consumed\nHello, \xe9\x9b\xaa\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        expected
    );
    let output = tempfile::tempdir().unwrap();
    let binary = output.path().join("response-app");
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
        ok(Command::new(moved)
            .current_dir(elsewhere.path())
            .env_clear()
            .output()
            .unwrap()),
        expected
    );
}
