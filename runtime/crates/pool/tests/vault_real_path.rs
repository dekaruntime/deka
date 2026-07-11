use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use harar_client::VaultClient;
use pool::{ExecutionMode, HandlerKey, IsolatePool, PoolConfig, RequestData, RequestParts};
use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)]
async fn vault_secrets_are_loaded_from_real_gild_vault_per_shop_isolate() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let vault_bin = vault_bin();
    let dir = TempDir::new().expect("tempdir");
    let socket = dir.path().join("gild-vault.sock");
    let state_path = dir.path().join("keys.json");
    let audit_log = dir.path().join("audit.log");
    let mut vault = spawn_vault_agent(&vault_bin, &socket, &state_path, &audit_log);
    wait_for_socket(&socket);

    let client = VaultClient::from_socket_path(&socket);
    client
        .put("shops/shop_a/SECRET", "foo")
        .await
        .expect("seed shop A secret");
    client
        .put("shops/shop_b/SECRET", "bar")
        .await
        .expect("seed shop B secret");

    let previous_socket = std::env::var_os("GILD_VAULT_SOCKET");
    unsafe {
        std::env::set_var("GILD_VAULT_SOCKET", &socket);
    }

    let pool = IsolatePool::new(
        PoolConfig {
            num_workers: 1,
            max_isolates_per_worker: 4,
            enable_code_cache: false,
            ..PoolConfig::default()
        },
        Arc::new(Vec::new),
    );
    let handler_code = vault_handler_code();

    let shop_a = execute_shop(&pool, "shop_a", &handler_code).await;
    let shop_b = execute_shop(&pool, "shop_b", &handler_code).await;

    restore_env("GILD_VAULT_SOCKET", previous_socket);
    let _ = vault.kill();
    let _ = vault.wait();

    assert_eq!(shop_a["shop"], "shop_a");
    assert_eq!(shop_a["env_secret"], "foo");
    assert_eq!(shop_a["process_secret"], "foo");
    assert_eq!(shop_a["vault_value"], "foo");
    assert_eq!(shop_a["crafted_ok"], false);
    assert_ne!(shop_a["crafted_value"], "bar");
    assert_eq!(shop_a["env_contains_shop_b_value"], false);

    assert_eq!(shop_b["shop"], "shop_b");
    assert_eq!(shop_b["env_secret"], "bar");
    assert_eq!(shop_b["process_secret"], "bar");
    assert_eq!(shop_b["vault_value"], "bar");

    // Avoid V8 isolate teardown during test-process shutdown; this test's
    // assertions are about request-time tenant scoping through the pool.
    std::mem::forget(pool);
}

async fn execute_shop(pool: &IsolatePool, shop_id: &str, handler_code: &str) -> serde_json::Value {
    let response = pool
        .execute(
            HandlerKey::new(format!("vault-real-path-{shop_id}")),
            RequestData {
                handler_code: handler_code.to_string(),
                handler_entry: None,
                request_value: serde_json::json!({}),
                request_parts: Some(RequestParts {
                    url: "http://localhost/vault-test".to_string(),
                    method: "GET".to_string(),
                    headers: vec![("host".to_string(), format!("{shop_id}.tana.gg"))],
                    body: None,
                }),
                mode: ExecutionMode::Request,
            },
        )
        .await
        .expect("execute isolate request");

    assert!(response.success, "isolate failed: {:?}", response.error);
    assert!(
        !response.cache_hit,
        "distinct handler key should create a fresh isolate for {shop_id}"
    );
    let result = response.result.expect("isolate result");
    let body = result
        .get("body")
        .and_then(|value| value.as_str())
        .expect("response body");
    serde_json::from_str(body).expect("parse response body")
}

fn vault_handler_code() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf();
    let bridge = std::fs::read_to_string(root.join("php_modules/core/bridge.phpx"))
        .expect("read core bridge module");
    let vault = std::fs::read_to_string(root.join("php_modules/deka/vault/index.phpx"))
        .expect("read deka vault module");
    let source = format!(
        "{}\n{}\n",
        standalone_phpx(&bridge),
        standalone_phpx(&vault)
    );
    let mut js = phpx_js::compile_phpx_source_to_js(
        &source,
        "php_modules/deka/vault/real_path_test.phpx",
        phpx_js::SourceModuleMeta::empty(),
    )
    .expect("compile real deka/vault PHPX");
    js = js.replace("export const ", "const ");
    js.push_str(
        r#"
const app = {
  fetch() {
    const own = get('SECRET');
    const crafted = get('../B/SECRET');
    const env = globalThis._ENV || {};
    const processEnv = (globalThis.process && globalThis.process.env) || {};
    const server = globalThis._SERVER || {};
    return {
      status: 200,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        shop: server.SHOP_ID || globalThis.__shopId || '',
        env_secret: env.SECRET || '',
        process_secret: processEnv.SECRET || '',
        vault_value: own.value || '',
        crafted_ok: own.ok === true && crafted.ok === true,
        crafted_value: crafted.value || '',
        env_contains_shop_b_value: JSON.stringify(env).includes('bar') ||
          JSON.stringify(processEnv).includes('bar')
      })
    };
  }
};
"#,
    );
    js
}

fn standalone_phpx(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.replacen("export function ", "function ", 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn vault_bin() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_gild-vault") {
        return PathBuf::from(path);
    }

    let mut path = std::env::current_exe().expect("current test exe");
    path.pop();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.push("gild-vault");
    if !path.exists() {
        build_gild_vault_bin();
    }
    assert!(
        path.exists(),
        "missing {}; cargo build --release -p gild-vault did not produce the vault binary",
        path.display()
    );
    path
}

fn build_gild_vault_bin() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(cargo)
        .args(["build", "--release", "-p", "gild-vault"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("build gild-vault binary");
    assert!(
        status.success(),
        "cargo build --release -p gild-vault failed"
    );
}

fn spawn_vault_agent(bin: &Path, socket: &Path, state_path: &Path, audit_log: &Path) -> Child {
    Command::new(bin)
        .args([
            "--socket",
            socket.to_str().expect("socket path utf8"),
            "--state-path",
            state_path.to_str().expect("state path utf8"),
            "--audit-log",
            audit_log.to_str().expect("audit log path utf8"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn gild-vault")
}

fn wait_for_socket(socket: &Path) {
    for _ in 0..200 {
        if socket.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("vault socket did not appear at {}", socket.display());
}

fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
    unsafe {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}
