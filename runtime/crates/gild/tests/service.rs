use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn gild_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gild")
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "{}-{}-{}",
            prefix,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create test dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum VaultRequest {
    Put { key: String, value: String },
    List,
    Delete { key: String },
}

fn spawn_mock_vault(
    socket_path: PathBuf,
    expected_requests: usize,
) -> (thread::JoinHandle<()>, Arc<Mutex<BTreeMap<String, String>>>) {
    let store = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
    let server_store = Arc::clone(&store);
    let handle = thread::spawn(move || {
        let listener = UnixListener::bind(&socket_path).expect("bind mock vault");
        listener
            .set_nonblocking(true)
            .expect("set mock vault nonblocking");

        for _ in 0..expected_requests {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(err) => panic!("accept mock vault request: {err}"),
                }
            };

            let mut body = Vec::new();
            stream.read_to_end(&mut body).expect("read vault request");
            let request: VaultRequest =
                serde_json::from_slice(&body).expect("parse vault request JSON");
            let response = match request {
                VaultRequest::Put { key, value } => {
                    server_store.lock().unwrap().insert(key, value);
                    serde_json::json!({ "ok": true })
                }
                VaultRequest::List => {
                    let keys = server_store
                        .lock()
                        .unwrap()
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>();
                    serde_json::json!({ "ok": true, "keys": keys })
                }
                VaultRequest::Delete { key } => {
                    server_store.lock().unwrap().remove(&key);
                    serde_json::json!({ "ok": true })
                }
            };
            stream
                .write_all(response.to_string().as_bytes())
                .expect("write vault response");
        }
    });
    (handle, store)
}

#[test]
fn service_link_ls_unlink_flow_uses_vault_socket() {
    let dir = TestDir::new("gild-service-flow");
    let socket = dir.path().join("vault.sock");
    let auth = dir.path().join("codex-auth.json");
    fs::write(&auth, r#"{"token":"codex-test"}"#).expect("write auth fixture");
    let (server, store) = spawn_mock_vault(socket.clone(), 7);

    let link_auth = Command::new(gild_bin())
        .args([
            "service",
            "link",
            "--agent",
            "agent-amina",
            "--provider",
            "codex",
            "--from",
            auth.to_str().unwrap(),
        ])
        .env("GILD_VAULT_SOCKET", &socket)
        .output()
        .expect("run service link auth");
    assert!(
        link_auth.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&link_auth.stderr)
    );
    let stdout = String::from_utf8_lossy(&link_auth.stdout);
    assert!(
        stdout.contains("linked codex auth for agent-amina"),
        "{stdout}"
    );
    assert!(stdout.contains("AGENT_AMINA_CODEX_AUTH"), "{stdout}");
    assert!(stdout.contains("sha256 "), "{stdout}");

    let link_token = Command::new(gild_bin())
        .args([
            "service",
            "link",
            "--agent",
            "agent-amina",
            "--provider",
            "claude",
            "--token",
            "sk-ant-test",
        ])
        .env("GILD_VAULT_SOCKET", &socket)
        .output()
        .expect("run service link token");
    assert!(
        link_token.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&link_token.stderr)
    );

    let ls = Command::new(gild_bin())
        .args(["service", "ls", "--agent", "agent-amina"])
        .env("GILD_VAULT_SOCKET", &socket)
        .output()
        .expect("run service ls");
    assert!(ls.status.success());
    let stdout = String::from_utf8_lossy(&ls.stdout);
    assert!(stdout.contains("agent-amina codex"), "{stdout}");
    assert!(stdout.contains("AGENT_AMINA_CODEX_AUTH"), "{stdout}");
    assert!(stdout.contains("agent-amina claude"), "{stdout}");
    assert!(stdout.contains("AGENT_AMINA_CLAUDE_TOKEN"), "{stdout}");

    let unlink = Command::new(gild_bin())
        .args([
            "service",
            "unlink",
            "--agent",
            "agent-amina",
            "--provider",
            "codex",
        ])
        .env("GILD_VAULT_SOCKET", &socket)
        .output()
        .expect("run service unlink");
    assert!(
        unlink.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&unlink.stderr)
    );
    let stdout = String::from_utf8_lossy(&unlink.stdout);
    assert!(
        stdout.contains("unlinked codex for agent-amina"),
        "{stdout}"
    );

    let ls_after = Command::new(gild_bin())
        .args(["service", "ls", "--agent", "agent-amina"])
        .env("GILD_VAULT_SOCKET", &socket)
        .output()
        .expect("run service ls after unlink");
    assert!(ls_after.status.success());
    let stdout = String::from_utf8_lossy(&ls_after.stdout);
    assert!(!stdout.contains("AGENT_AMINA_CODEX_AUTH"), "{stdout}");
    assert!(stdout.contains("AGENT_AMINA_CLAUDE_TOKEN"), "{stdout}");

    let unlink_missing = Command::new(gild_bin())
        .args([
            "service",
            "unlink",
            "--agent",
            "agent-amina",
            "--provider",
            "codex",
        ])
        .env("GILD_VAULT_SOCKET", &socket)
        .output()
        .expect("run service unlink missing");
    assert!(unlink_missing.status.success());

    server.join().expect("mock vault thread");
    let store = store.lock().unwrap();
    assert!(!store.contains_key("AGENT_AMINA_CODEX_AUTH"));
    assert_eq!(
        store.get("AGENT_AMINA_CLAUDE_TOKEN").map(String::as_str),
        Some("sk-ant-test")
    );
}

#[test]
fn service_link_rejects_invalid_slug_before_socket_connect() {
    let output = Command::new(gild_bin())
        .args([
            "service",
            "link",
            "--agent",
            "agent-1bad",
            "--provider",
            "codex",
            "--token",
            "token",
        ])
        .env("GILD_VAULT_SOCKET", "/tmp/gild-service-missing.sock")
        .output()
        .expect("run service link");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("agent slug must match"), "{stderr}");
    assert!(!stderr.contains("No such file"), "{stderr}");
}

#[test]
fn service_help_pages_render() {
    for args in [
        ["service", "--help"].as_slice(),
        ["service", "link", "--help"].as_slice(),
        ["service", "ls", "--help"].as_slice(),
        ["service", "unlink", "--help"].as_slice(),
    ] {
        let output = Command::new(gild_bin()).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage:"), "{stdout}");
    }
}
