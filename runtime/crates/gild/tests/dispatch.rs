use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn gild_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gild")
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn command_with_agent_fixture() -> Command {
    let fixture = fixture_path("agent-ports-mini.json");
    let mut command = Command::new(gild_bin());
    command
        .env("AGENT_PORTS_FILE", &fixture)
        .env("GILD_AGENT_PORTS_CONFIG", fixture)
        .env("GILD_SYSTEMCTL_BIN", "/bin/false");
    command
}

#[test]
fn dispatch_to_real_agent_returns_run_id() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock dispatcher");
    listener
        .set_nonblocking(true)
        .expect("set dispatcher nonblocking");
    let port = listener.local_addr().expect("dispatcher addr").port();
    let (request_tx, request_rx) = mpsc::channel();

    let server = thread::spawn(move || {
        let started = Instant::now();
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(accepted) => break accepted,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "timed out waiting for dispatch request"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("accept dispatch request: {err}"),
            }
        };

        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        let header_end = loop {
            let read = stream.read(&mut buffer).expect("read dispatch request");
            assert!(read > 0, "dispatcher request closed before headers");
            request.extend_from_slice(&buffer[..read]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length:")
                    .or_else(|| line.strip_prefix("Content-Length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .expect("request content-length");
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer).expect("read dispatch body");
            assert!(read > 0, "dispatcher request closed before body");
            request.extend_from_slice(&buffer[..read]);
        }

        let request = String::from_utf8_lossy(&request).to_string();
        request_tx.send(request).expect("record dispatch request");

        let body = r#"{"run_id":"run_12345abc"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write dispatch response");
    });

    let dir = std::env::temp_dir().join(format!(
        "gild-dispatch-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("create test dir");
    let config = dir.join("agents.toml");
    fs::write(
        &config,
        format!(
            r#"
[agents.agent-test]
port = {port}
name = "Test Agent"
sandbox = "codex"
"#
        ),
    )
    .expect("write agents config");

    let output = Command::new(gild_bin())
        .args(["dispatch", "agent-test", "echo OK", "--runtime", "codex"])
        .env("GILD_AGENTS_CONFIG", &config)
        .env("GILD_DISPATCH_SECRET", "dispatch-test-secret")
        .env_remove("GILD_DISPATCH_SECRET_FILE")
        .env_remove("GILD_DISPATCH_SECRET_TEST")
        .output()
        .expect("spawn gild dispatch");

    let request = request_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| {
            panic!(
                "dispatcher did not receive request; status={:?}, stderr={}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            )
        });
    server.join().expect("mock dispatcher thread");

    assert!(
        output.status.success(),
        "stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run_id = String::from_utf8(output.stdout)
        .expect("stdout utf8")
        .trim()
        .to_string();
    assert!(run_id.starts_with("run_"), "run_id was: {run_id}");
    assert!(
        run_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
        "run_id was: {run_id}"
    );

    assert!(request.starts_with("POST /task HTTP/1.1"), "{request}");
    assert!(request.contains("x-gild-timestamp:"), "{request}");
    assert!(request.contains("x-gild-signature:"), "{request}");
    assert!(request.contains(r#""task":"echo OK""#), "{request}");
    assert!(request.contains(r#""runtime":"codex""#), "{request}");
}

#[test]
fn dispatch_with_missing_agent_returns_clear_error() {
    let output = command_with_agent_fixture()
        .args(["dispatch", "agent-doesnt-exist", "task"])
        .output()
        .expect("spawn gild");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("unknown agent"),
        "stderr was: {stderr}"
    );
}

#[test]
fn dispatch_with_invalid_runtime_returns_clear_error() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gild"))
        .args(["dispatch", "agent-tariq", "task", "--runtime", "nonsense"])
        .output()
        .expect("spawn gild");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Assert the error came from clap's enum validation, not a connection error
    assert!(
        stderr.contains("invalid value")
            || stderr.contains("possible values")
            || stderr.contains("unknown runtime"),
        "expected runtime validation error, got: {}",
        stderr
    );
    assert!(
        !stderr.contains("Connection refused"),
        "test fell through to network call"
    );
}

#[test]
fn agent_ls_prints_configured_agents() {
    let output = command_with_agent_fixture()
        .args(["agent", "ls"])
        .output()
        .expect("spawn gild");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("agent-test1"));
    assert!(stdout.contains("agent-test2"));
}
