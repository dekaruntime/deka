use std::process::{Command, Output};
#[path = "../../deka_vm/tests/support/http_server.rs"]
mod server;
use server::{Server, reply};
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
fn real_native_fetch_checks_runs_and_survives_source_deletion_and_relocation() {
    let server = Server::new(|path, _| {
        Ok(match path {
            "/json" => reply(
                "200 OK",
                "Content-Type: application/json\r\n",
                br#""network Deka""#,
            ),
            _ => reply("404 Not Found", "", b"missing"),
        })
    })
    .unwrap();
    let project = tempfile::tempdir().unwrap();
    let guide = include_str!("../../../docs/dekascript/native/fetch.mdx");
    let examples = guide
        .split("```ds\n")
        .skip(1)
        .map(|block| block.split("```").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(examples.len(), 2);
    let readers = examples.join("\n");
    std::fs::write(
        project.path().join("main.ds"),
        format!(
            r#"import {{echo}} from "io";
{readers}
async fn main() {{
    echo(await loadTitle("{}"));
    echo(await loadPair("{}","{}"));
    const missing=unwrap(await fetch("{}")) or{{echo("missing fetch failed");return;}};
    echo(string(missing.status));
    echo(string(missing.ok));
    const body=unwrap(await missing.text()) or{{echo("read failed");return;}};
    echo(body);
}}"#,
            server.url("/json"),
            server.url("/json"),
            server.url("/missing"),
            server.url("/missing")
        ),
    )
    .unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    ok(Command::new(cli)
        .args(["check", "main.ds", "--entry", "main"])
        .current_dir(project.path())
        .output()
        .unwrap());
    let expected = b"network Deka\nHTTP 200; HTTP 404\n404\nfalse\nmissing\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        expected
    );
    let output = tempfile::tempdir().unwrap();
    let binary = output.path().join("fetch-app");
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
    let requests = server.finish().unwrap();
    assert_eq!(requests.len(), 8);
    assert_eq!(requests.iter().filter(|path| *path == "/json").count(), 4);
    assert_eq!(
        requests.iter().filter(|path| *path == "/missing").count(),
        4
    );
}
