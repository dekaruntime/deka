#[path = "support/check.rs"]
mod check;
#[path = "../../deka_vm/tests/support/http_server.rs"]
mod server;
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
fn actual_cli_network_and_parser_survive_source_deletion_and_relocation() {
    let server =
        server::Server::new(|path, _| Ok(server::reply("201 Created", "", path.as_bytes())))
            .unwrap();
    let project = tempfile::tempdir().unwrap();
    let source = format!(
        r#"import {{get as load,parse_url}} from "http";
    import {{to_string}} from "bytes";
    import {{echo}} from "io";
    fn main() {{
        echo(match load("{}"){{Ok(r)=>string(r.status)+":"+(match to_string(r.body){{Ok(s)=>s,Err(e)=>e}}),Err(e)=>e}});
        echo(match parse_url("http://"){{Ok(u)=>"bad",Throw(e)=>e.message}});
    }}"#,
        server.url("/live")
    );
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
    let expected = b"201:/live\ninvalid url\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        expected
    );
    let destination = tempfile::tempdir().unwrap();
    let binary = destination.path().join("http-app");
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
    assert_eq!(server.finish().unwrap(), ["/live", "/live"]);
}
#[test]
fn http_documentation_checks_runs_and_packages_without_legacy_helpers() {
    let guide = include_str!("../../../docs/dekascript/native/http-module.mdx");
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
    check::checked(
        Command::new(cli)
            .args(["check", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap(),
        "main.ds",
    );
    let out = ok(Command::new(cli)
        .args(["run", "main.ds", "--entry", "main"])
        .current_dir(project.path())
        .output()
        .unwrap());
    assert_eq!(out,b"example.com\nGET /projects HTTP/1.1\r\nContent-Type: \r\nHost: example.com\r\nContent-Length: 0\r\nConnection: close\r\n\r\n\nnetwork error\n");
}
