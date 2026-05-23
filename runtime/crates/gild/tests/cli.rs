use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

fn gild_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gild")
}

#[test]
fn version_prints() {
    let output = Command::new(gild_bin()).arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("gild"));
}

#[test]
fn agent_ls_reads_config_file() {
    let dir = std::env::temp_dir().join(format!(
        "gild-cli-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let config = dir.join("agents.toml");
    fs::write(
        &config,
        r#"
[agents.agent-khalid]
port = 9434
name = "Khalid"
sandbox = "gild"
"#,
    )
    .unwrap();

    let output = Command::new(gild_bin())
        .args(["agent", "ls"])
        .env("GILD_AGENTS_CONFIG", &config)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("agent-khalid"));
    assert!(stdout.contains("9434"));
    assert!(stdout.contains("Khalid"));
}

#[test]
fn dispatch_sets_hmac_headers_with_configured_secret() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let len = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..len]);
        assert!(request.starts_with("POST /task HTTP/1.1"), "{request}");
        assert!(request.contains("x-gild-timestamp:"), "{request}");
        assert!(request.contains("x-gild-signature:"), "{request}");
        assert!(request.contains(r#""task":"fix the thing""#), "{request}");

        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 26\r\n\r\n{\"run_id\":\"run-from-test\"}",
            )
            .unwrap();
    });

    let dir = std::env::temp_dir().join(format!(
        "gild-cli-dispatch-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let config = dir.join("agents.toml");
    fs::write(
        &config,
        format!(
            r#"
[agents.agent-khalid]
port = {port}
name = "Khalid"
sandbox = "gild"
"#
        ),
    )
    .unwrap();

    let output = Command::new(gild_bin())
        .args(["dispatch", "agent-khalid", "fix the thing"])
        .env("GILD_AGENTS_CONFIG", &config)
        .env("GILD_DISPATCH_SECRET", "integration-test-secret")
        .env_remove("GILD_DISPATCH_SECRET_FILE")
        .env_remove("GILD_DISPATCH_SECRET_TEST")
        .output()
        .unwrap();

    server.join().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("run-from-test"));
}
