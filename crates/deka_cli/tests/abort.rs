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
fn abort_guide_checks_runs_and_survives_source_deletion_and_relocation() {
    use std::io::Read;
    let gate = std::sync::Arc::new((
        std::sync::Mutex::new((0usize, 0usize)),
        std::sync::Condvar::new(),
    ));
    let server = Server::new(move |path, stream| {
        let (state, changed) = &*gate;
        match path {
            "/actual-cancel" => {
                state
                    .lock()
                    .map_err(|_| std::io::Error::other("poisoned arrival gate"))?
                    .0 += 1;
                changed.notify_all();
                let mut byte = [0];
                match stream.read(&mut byte) {
                    Ok(0) => Ok(vec![]),
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => Ok(vec![]),
                    Ok(_) => Err(std::io::Error::other(
                        "cancelled CLI request stayed connected",
                    )),
                    Err(e) => Err(e),
                }
            }
            "/barrier" => {
                let mut counts = state
                    .lock()
                    .map_err(|_| std::io::Error::other("poisoned barrier"))?;
                counts.1 += 1;
                let target = counts.1;
                while counts.0 < target {
                    let (next, timeout) = changed
                        .wait_timeout(counts, std::time::Duration::from_secs(5))
                        .map_err(|_| std::io::Error::other("poisoned barrier wait"))?;
                    counts = next;
                    if timeout.timed_out() && counts.0 < target {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "cancelled request never arrived",
                        ));
                    }
                }
                Ok(reply("200 OK", "", b"ready"))
            }
            "/fail" => Ok(
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort"
                    .to_vec(),
            ),
            "/other" => Ok(reply("200 OK", "", br#""still running""#)),
            _ => Err(std::io::Error::other(
                "pre-aborted request unexpectedly reached the server",
            )),
        }
    })
    .unwrap();
    let guide = include_str!("../../../docs/dekascript/native/abort.mdx");
    let examples = guide
        .split("```ds\n")
        .skip(1)
        .map(|b| b.split("```").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(examples.len(), 2);
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("main.ds"),
        format!(
            r#"import {{echo}} from "io";
{}
async fn main(){{
    const controller=AbortController();
    const cancelled=fetch("{}",{{signal:Some(controller.signal)}});
    const other=fetch("{}"); const failed=fetch("{}");
    const barrier=unwrap(await fetch("{}")) or{{echo("barrier failed");return;}};
    controller.abort("One cancelled");
    echo(match await cancelled{{Ok(r)=>"unexpected",Err(e)=>e}});
    echo(match await failed{{Ok(r)=>"unexpected",Err(e)=>"Failed"}});
    echo(await cancelOne("{}")); echo(await withDeadline("{}"));
    const response=unwrap(await other) or{{echo("other failed");return;}};
    const value=unwrap(await response.json<string>()) or{{return;}}; echo(value);
}}"#,
            examples.join("\n"),
            server.url("/actual-cancel"),
            server.url("/other"),
            server.url("/fail"),
            server.url("/barrier"),
            server.url("/cancel"),
            server.url("/timeout")
        ),
    )
    .unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    ok(Command::new(cli)
        .args(["check", "main.ds", "--entry", "main"])
        .current_dir(project.path())
        .output()
        .unwrap());
    let expected = b"One cancelled\nFailed\nStopped\nTimeoutError\nstill running\n";
    assert_eq!(
        ok(Command::new(cli)
            .args(["run", "main.ds", "--entry", "main"])
            .current_dir(project.path())
            .output()
            .unwrap()),
        expected
    );
    let out = tempfile::tempdir().unwrap();
    let binary = out.path().join("abort-app");
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
    let requests = server.finish().unwrap();
    assert_eq!(requests.len(), 8);
    for path in ["/actual-cancel", "/barrier", "/fail", "/other"] {
        assert_eq!(requests.iter().filter(|p| p.as_str() == path).count(), 2);
    }
}
