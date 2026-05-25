use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use age::secrecy::ExposeSecret;

fn gild_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gild")
}

fn vault_bin() -> PathBuf {
    if let Ok(path) = std::env::var("GILD_VAULT_BIN") {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_gild-vault") {
        return PathBuf::from(path);
    }

    let mut path = PathBuf::from(gild_bin());
    path.pop();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.push("gild-vault");
    build_gild_vault_bin(&path);
    path
}

fn build_gild_vault_bin(path: &Path) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(cargo)
        .args(["build", "--release", "-p", "gild-vault"])
        .current_dir(workspace)
        .status()
        .expect("build gild-vault binary");
    assert!(
        status.success(),
        "cargo build --release -p gild-vault failed"
    );
    assert!(
        path.exists(),
        "missing {}; cargo build --release -p gild-vault did not produce the vault binary",
        path.display()
    );
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
    Get { key: String },
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
                VaultRequest::Get { key } => match server_store.lock().unwrap().get(&key) {
                    Some(value) => serde_json::json!({ "ok": true, "value": value }),
                    None => serde_json::json!({ "ok": false, "error": "not_found" }),
                },
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
fn service_link_round_trips_against_real_vault_agent_socket() {
    let vault_bin = vault_bin();

    let dir = TestDir::new("gild-service-real-vault");
    let socket = dir.path().join("vault.sock");
    let store_path = dir.path().join("secrets.age");
    let audit_log = dir.path().join("audit.log");
    let master_key = dir.path().join("vault-master.key");
    let replication_token = dir.path().join("vault-replication-token");
    write_master_key(&master_key);
    fs::write(&replication_token, "test-replication-token\n").expect("write replication token");

    let mut vault = spawn_vault_agent(
        &vault_bin,
        &socket,
        &store_path,
        &audit_log,
        &master_key,
        &replication_token,
    );
    wait_for_socket(&socket);

    let output = Command::new(gild_bin())
        .args([
            "service",
            "link",
            "--agent",
            "agent-amina",
            "--provider",
            "codex",
            "--token",
            "sk-real-roundtrip",
        ])
        .env("GILD_VAULT_SOCKET", &socket)
        .output()
        .expect("run service link");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let value = read_secret_json(&socket, "AGENT_AMINA_CODEX_TOKEN");
    assert_eq!(value, "sk-real-roundtrip");

    let _ = vault.kill();
    let _ = vault.wait();
}

fn spawn_vault_agent(
    bin: &Path,
    socket: &Path,
    state_path: &Path,
    audit_log: &Path,
    master_key: &Path,
    replication_token: &Path,
) -> Child {
    Command::new(bin)
        .args([
            "--socket",
            socket.to_str().expect("socket path utf8"),
            "--state-path",
            state_path.to_str().expect("state path utf8"),
            "--audit-log",
            audit_log.to_str().expect("audit log path utf8"),
        ])
        .env("GILD_VAULT_MASTER_KEY_PATH", master_key)
        .env("GILD_VAULT_REPLICATION_TOKEN_FILE", replication_token)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn vault agent")
}

fn write_master_key(path: &Path) {
    let identity = age::x25519::Identity::generate();
    fs::write(path, format!("{}\n", identity.to_string().expose_secret()))
        .expect("write test master key");
    let mut perms = fs::metadata(path)
        .expect("stat test master key")
        .permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o400);
    fs::set_permissions(path, perms).expect("chmod test master key");
}

fn wait_for_socket(socket: &Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if socket.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("vault socket did not appear at {}", socket.display());
}

fn read_secret_json(socket: &Path, key: &str) -> String {
    let mut stream = std::os::unix::net::UnixStream::connect(socket).expect("connect vault");
    write!(stream, r#"{{"op":"get","key":"{key}"}}"#).expect("write JSON request");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read JSON response");
    serde_json::from_str::<serde_json::Value>(&response)
        .expect("secret JSON")
        .get("value")
        .and_then(|value| value.as_str())
        .expect("value field")
        .to_string()
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
