use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
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
        .env("GILD_SYSTEMCTL_BIN", "/bin/false")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("agent-khalid"));
    assert!(stdout.contains("9434"));
    assert!(stdout.contains("Khalid"));
    assert!(stdout.contains("inactive"));
}

#[test]
fn agent_ls_json_reads_mocked_systemctl() {
    let dir = std::env::temp_dir().join(format!(
        "gild-cli-ls-json-test-{}",
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
[agents.agent-zed]
port = 9444
name = "Zed"
sandbox = "vm"
"#,
    )
    .unwrap();
    let systemctl = dir.join("systemctl");
    fs::write(
        &systemctl,
        "#!/bin/sh\nif [ \"$1\" = \"is-active\" ]; then echo active; exit 0; fi\nexit 1\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&systemctl).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&systemctl, permissions).unwrap();

    let output = Command::new(gild_bin())
        .args(["agent", "ls", "--json"])
        .env("GILD_AGENTS_CONFIG", &config)
        .env("GILD_SYSTEMCTL_BIN", &systemctl)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value[0]["slug"], "agent-zed");
    assert_eq!(value[0]["port"], 9444);
    assert_eq!(value[0]["sandbox"], "vm");
    assert_eq!(value[0]["persona"], "Zed");
    assert_eq!(value[0]["active"], "active");
}

#[test]
fn agent_ls_missing_registry_is_clear_success() {
    let missing = std::env::temp_dir().join(format!(
        "gild-missing-agents-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = Command::new(gild_bin())
        .args(["agent", "ls"])
        .env("GILD_AGENTS_CONFIG", &missing)
        .env("GILD_AGENT_PORTS_CONFIG", &missing)
        .env("GILD_SYSTEMCTL_BIN", "/bin/false")
        .env("HOME", missing.parent().unwrap())
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("no registry found"));
}

#[test]
fn agent_create_help_shows_dry_run() {
    let output = Command::new(gild_bin())
        .args(["agent", "create", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Create a new agent"));
    assert!(stdout.contains("--dry-run"));
}

#[test]
fn agent_subcommands_show_help() {
    for args in [
        ["agent", "enable", "--help"],
        ["agent", "disable", "--help"],
        ["agent", "delete", "--help"],
        ["agent", "ls", "--help"],
    ] {
        let output = Command::new(gild_bin()).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("Usage:"), "{stdout}");
    }
}

#[test]
fn agent_create_dry_run_does_not_need_socket() {
    let output = Command::new(gild_bin())
        .args(["agent", "create", "agent-zed", "--dry-run"])
        .env("GILD_AGENT_SOCKET", "/tmp/gild-agent-test-missing.sock")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("dry run: would create agent agent-zed"));
    assert!(stdout.contains("useradd agent-zed"));
    assert!(stdout.contains("hmac_rotate agent-zed"));
    assert!(stdout.contains("gg.tana.gild-dispatcher@agent-zed.service"));
    assert!(stdout.contains("Environment=AGENT_SLUG=agent-zed"));
}

#[test]
fn agent_create_rejects_invalid_slug_before_socket() {
    let output = Command::new(gild_bin())
        .args(["agent", "create", "agent-1bad"])
        .env("GILD_AGENT_SOCKET", "/tmp/gild-agent-test-missing.sock")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("slug must match"), "{stderr}");
    assert!(!stderr.contains("No such file"), "{stderr}");
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
