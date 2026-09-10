use pool::{ExecutionMode, HandlerKey, IsolatePool, PoolConfig, RequestData, RequestParts};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn test_pool() -> IsolatePool {
    let config = PoolConfig {
        num_workers: 1,
        max_isolates_per_worker: 2,
        idle_timeout_secs: 30,
        enable_metrics: false,
        enable_code_cache: false,
        request_timeout_ms: 10_000,
        queue_timeout_ms: 10_000,
        ..PoolConfig::default()
    };
    IsolatePool::new(config, Arc::new(Vec::new))
}

/// Pool whose isolates enforce an explicit net policy instead of reading
/// `DEKA_SECURITY_POLICY` from the process env per dispatch. Tests pass
/// their own policy so parallel tests stop racing on the process-global
/// env var (deka#537).
fn net_test_pool(policy: runtime_core::security_policy::SecurityPolicy) -> IsolatePool {
    let config = PoolConfig {
        num_workers: 1,
        max_isolates_per_worker: 2,
        idle_timeout_secs: 30,
        enable_metrics: false,
        enable_code_cache: false,
        request_timeout_ms: 10_000,
        queue_timeout_ms: 10_000,
        ..PoolConfig::default()
    };
    IsolatePool::new(
        config,
        Arc::new(move || {
            platform_server::extensions_for_php_server_with_net_policy(policy.clone())
        }),
    )
}

fn allow_net_policy(target: &str) -> runtime_core::security_policy::SecurityPolicy {
    let document = serde_json::json!({
        "security": { "allow": { "net": [target] } }
    });
    runtime_core::security_policy::parse_deka_security_policy(&document).policy
}

/// Pool backed by the platform-server (php) extensions with default
/// env-driven policy. For tests that never touch the net bridge.
fn php_pool() -> IsolatePool {
    let config = PoolConfig {
        num_workers: 1,
        max_isolates_per_worker: 2,
        idle_timeout_secs: 30,
        enable_metrics: false,
        enable_code_cache: false,
        request_timeout_ms: 10_000,
        queue_timeout_ms: 10_000,
        ..PoolConfig::default()
    };
    IsolatePool::new(config, Arc::new(platform_server::extensions_for_php_server))
}

fn test_request(handler_code: &str) -> RequestData {
    RequestData {
        handler_code: handler_code.to_string(),
        handler_entry: None,
        module_root: None,
        request_value: serde_json::Value::Null,
        request_parts: None,
        mode: ExecutionMode::Request,
        security: None,
    }
}

fn tenant_request(handler_code: &str, shop_id: &str) -> RequestData {
    RequestData {
        handler_code: handler_code.to_string(),
        handler_entry: None,
        module_root: None,
        request_value: serde_json::Value::Null,
        request_parts: Some(RequestParts {
            url: "http://localhost/bridge".to_string(),
            method: "GET".to_string(),
            headers: vec![("host".to_string(), format!("{shop_id}.tana.gg"))],
            body: None,
        }),
        mode: ExecutionMode::Request,
        security: None,
    }
}

#[tokio::test]
async fn host_dispatchers_are_not_on_user_globalthis() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const internal = globalThis[Symbol.for('deka.host.internal')] || {};
  return {
    status: 200,
    headers: {},
    body: JSON.stringify({
      bridge: typeof globalThis.__bridge,
      bridgeAsync: typeof globalThis.__bridge_async,
      wasmCall: typeof globalThis.__deka_wasm_call,
      wasmCallAsync: typeof globalThis.__deka_wasm_call_async,
      host: typeof globalThis.__deka_host,
      deno: typeof globalThis.Deno,
      closedHost: typeof __deka_host,
      closedBridge: typeof __bridge,
      internalFrozen: Object.isFrozen(internal),
      internalHost: typeof internal.host,
      internalBridge: typeof internal.bridge,
      internalToResult: typeof internal.toResult,
      internalOps: typeof internal.ops,
      internalBridgeAsync: typeof internal.bridgeAsync,
      internalWasmCall: typeof internal.wasmCall,
      internalWasmCallAsync: typeof internal.wasmCallAsync
    })
  };
};
"#;
    let res = pool
        .execute(HandlerKey::new("host_not_global"), test_request(code))
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["bridge"], "undefined", "body={body}");
    assert_eq!(parsed["bridgeAsync"], "undefined");
    assert_eq!(parsed["wasmCall"], "undefined");
    assert_eq!(parsed["wasmCallAsync"], "undefined");
    assert_eq!(parsed["host"], "undefined");
    assert_eq!(parsed["deno"], "undefined");
    assert_eq!(parsed["closedHost"], "function");
    assert_eq!(parsed["closedBridge"], "function");
    assert_eq!(parsed["internalFrozen"], true);
    assert_eq!(parsed["internalHost"], "function");
    assert_eq!(parsed["internalBridge"], "function");
    assert_eq!(parsed["internalToResult"], "function");
    assert_eq!(parsed["internalOps"], "undefined");
    assert_eq!(parsed["internalBridgeAsync"], "undefined");
    assert_eq!(parsed["internalWasmCall"], "undefined");
    assert_eq!(parsed["internalWasmCallAsync"], "undefined");
}

#[tokio::test]
async fn bridge_unknown_kind_returns_error_envelope() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const result = __bridge('unknown_kind', 'x', {});
  return { status: 200, headers: {}, body: JSON.stringify(result) };
};
"#;
    let res = pool
        .execute(HandlerKey::new("bridge_unknown_kind"), test_request(code))
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution should succeed");
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(
        parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("unknown bridge kind")
    );
}

#[tokio::test]
async fn bridge_unknown_action_returns_error_envelope() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const result = __bridge('crypto', 'unknown_action', {});
  return { status: 200, headers: {}, body: JSON.stringify(result) };
};
"#;
    let res = pool
        .execute(HandlerKey::new("bridge_unknown_action"), test_request(code))
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(
        parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("unknown crypto action")
    );
}

#[tokio::test]
async fn bridge_result_envelope_has_ok_shape() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const result = __bridge('time', 'now_ms', {});
  return { status: 200, headers: {}, body: JSON.stringify(result) };
};
"#;
    let res = pool
        .execute(HandlerKey::new("bridge_ok_shape"), test_request(code))
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    // The time bridge currently returns Object.entries({ok: true, now_ms: ...}),
    // which serializes as [["ok",true],["now_ms",...]]. We verify the ok field exists.
    assert!(
        body.contains("\"ok\""),
        "result should contain ok field: {}",
        body
    );
}

#[tokio::test]
async fn net_bridge_connects_through_isolate_as_entry_pairs() {
    const CONNECTS: usize = 24;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind TCP listener");
    listener
        .set_nonblocking(true)
        .expect("make TCP listener nonblocking");
    let port = listener.local_addr().expect("listener address").port();
    let accepted = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut count = 0;
        while count < CONNECTS && Instant::now() < deadline {
            match listener.accept() {
                Ok((_stream, _peer)) => count += 1,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept TCP connection: {error}"),
            }
        }
        count
    });

    let pool = net_test_pool(allow_net_policy(&format!("127.0.0.1:{port}")));
    let code = format!(
        r#"
globalThis.app = function(req) {{
  // The raw __bridge op returns a plain object ({{ok, handle}} / {{ok, error}}),
  // not an entries array -- PHPX's stdlib (tcp/index.phpx's to_assoc()) accepts
  // this shape directly via a gettype()==='object' branch rather than the
  // bridge being made to emit Object.entries pairs. Exercise the real shape.
  const connects = [];
  for (let i = 0; i < {CONNECTS}; i++) {{
    const raw = __bridge('net', 'connect', {{ host: '127.0.0.1', port: {port} }});
    // The raw __bridge op returns entry pairs ([[key,value],...]); PHPX's
    // stdlib (tcp/index.phpx's to_assoc()) converts this to an assoc array.
    // Exercise the real wire shape here rather than a plain object.
    if (!Array.isArray(raw)) throw new Error(`net bridge result ${{i}} was not entry pairs: ${{JSON.stringify(raw)}}`);
    const result = Object.fromEntries(raw);
    if (result.ok !== true || !Number.isInteger(result.handle) || result.handle < 1) {{
      throw new Error(`invalid net bridge response ${{JSON.stringify(raw)}}`);
    }}
    connects.push(result.handle);
  }}
  const closes = connects.map((handle) => Object.fromEntries(__bridge('net', 'close', {{ handle }})));
  if (!closes.every((result) => result.ok === true)) throw new Error('net bridge close failed');
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{ connects: connects.length }}) }};
}};
"#
    );
    let res = pool
        .execute(
            HandlerKey::new("net_bridge_tcp_connect"),
            test_request(&code),
        )
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    assert_eq!(body, format!(r#"{{"connects":{CONNECTS}}}"#));
    assert_eq!(accepted.join().expect("TCP listener thread"), CONNECTS);
}

/// A numeric network handle is an isolate-local capability, not a process-wide
/// identifier.  Use distinct handler keys and shop request contexts so the
/// pool creates the two independent user isolates used in production.
#[tokio::test]
async fn net_bridge_rejects_foreign_handles_across_tenant_isolates() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind TCP listener");
    let port = listener.local_addr().expect("listener address").port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept tenant A connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        let mut byte = [0_u8; 1];
        std::io::Read::read(&mut stream, &mut byte).expect("wait for owner close")
    });

    let pool = net_test_pool(allow_net_policy(&format!("127.0.0.1:{port}")));
    let owner_code = format!(
        r#"
globalThis.app = function(req) {{
  if (!globalThis.__tenantANetHandle) {{
    const connect = Object.fromEntries(__bridge('net', 'connect', {{ host: '127.0.0.1', port: {port} }}));
    if (connect.ok !== true) throw new Error(`tenant A connect failed: ${{JSON.stringify(connect)}}`);
    globalThis.__tenantANetHandle = connect.handle;
    return {{ status: 200, headers: {{}}, body: JSON.stringify(connect) }};
  }}
  const close = Object.fromEntries(__bridge('net', 'close', {{ handle: globalThis.__tenantANetHandle }}));
  return {{ status: 200, headers: {{}}, body: JSON.stringify(close) }};
}};
"#
    );
    let owner_key = HandlerKey::new("net_owner_tenant_a");
    let connected = pool
        .execute(owner_key.clone(), tenant_request(&owner_code, "shop_owner"))
        .await
        .expect("tenant A execution");
    assert!(connected.success, "tenant A failed: {:?}", connected.error);
    let connected_body = connected
        .result
        .expect("tenant A result")
        .get("body")
        .and_then(|value| value.as_str())
        .expect("tenant A response body")
        .to_string();
    let handle = serde_json::from_str::<serde_json::Value>(&connected_body)
        .expect("parse tenant A response")
        .get("handle")
        .and_then(|value| value.as_u64())
        .expect("tenant A handle");

    let attacker_code = format!(
        r#"
globalThis.app = function(req) {{
  const handle = {handle};
  // Mirror PHPX's tcp/index.phpx to_assoc(): the raw bridge emits entry
  // pairs on the success path but can return a plain object on some error
  // paths (e.g. an "unknown handle" rejection for a foreign tenant) --
  // handle both shapes rather than assuming one universally.
  const toAssoc = (raw) => {{
    if (Array.isArray(raw)) return Object.fromEntries(raw);
    if (raw && typeof raw === 'object') return raw;
    return {{}};
  }};
  const attempt = (action, payload) => {{
    try {{
      return toAssoc(__bridge('net', action, payload));
    }} catch (error) {{
      return {{ ok: false, error: String(error) }};
    }}
  }};
  const actions = {{
    read: attempt('read', {{ handle, max_bytes: 1 }}),
    write: attempt('write', {{ handle, data: 'x' }}),
    close: attempt('close', {{ handle }}),
    tls_upgrade: attempt('tls_upgrade', {{ handle, server_name: 'localhost' }}),
  }};
  return {{ status: 200, headers: {{}}, body: JSON.stringify(actions) }};
}};
"#
    );
    let attacked = pool
        .execute(
            HandlerKey::new("net_attacker_tenant_b"),
            tenant_request(&attacker_code, "shop_attacker"),
        )
        .await
        .expect("tenant B execution");
    assert!(attacked.success, "tenant B failed: {:?}", attacked.error);
    let attacked_result = attacked.result.expect("tenant B result");
    let attack_body = attacked_result
        .get("body")
        .and_then(|value| value.as_str())
        .expect("tenant B response body");
    let actions: serde_json::Value = serde_json::from_str(attack_body).expect("parse attack body");
    for action in ["read", "write", "close", "tls_upgrade"] {
        assert_eq!(
            actions[action]["ok"],
            serde_json::json!(false),
            "tenant B must not {action} tenant A handle: {}",
            actions[action]
        );
        assert!(
            actions[action]["error"]
                .as_str()
                .is_some_and(|error| error.contains("unknown handle")),
            "foreign {action} must fail before socket I/O: {}",
            actions[action]
        );
    }

    let closed = pool
        .execute(owner_key, tenant_request(&owner_code, "shop_owner"))
        .await
        .expect("tenant A close execution");
    assert!(closed.success, "tenant A close failed: {:?}", closed.error);
    let closed_result = closed.result.expect("tenant A close result");
    let close_body = closed_result
        .get("body")
        .and_then(|value| value.as_str())
        .expect("tenant A close body");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(close_body).expect("parse tenant A close body")["ok"],
        serde_json::json!(true),
        "the owner must retain its handle after tenant B's attempts"
    );
    assert_eq!(
        server.join().expect("TCP server join"),
        0,
        "foreign write reached socket"
    );
}

#[tokio::test]
async fn bridge_redis_flush_is_blocked_before_native_dispatch() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const flush = __bridge('redis', 'flush', { handle: 1 });
  const flushdb = __bridge('redis', 'FLUSHDB', { handle: 1 });
  const flushall = __bridge('redis', 'flushall', { handle: 1 });
  return { status: 200, headers: {}, body: JSON.stringify({ flush, flushdb, flushall }) };
};
"#;
    let res = pool
        .execute(
            HandlerKey::new("bridge_redis_flush_blocked"),
            tenant_request(code, "shop_flush_guard"),
        )
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    for action in ["flush", "flushdb", "flushall"] {
        assert_eq!(
            parsed[action],
            serde_json::json!({
                "ok": false,
                "error": "redis admin action blocked for tenant code"
            }),
            "{action} should be rejected before FLUSHDB/FLUSHALL can reach Redis"
        );
    }
}

#[tokio::test]
async fn bridge_redis_unscoped_enumeration_verbs_are_blocked() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const scan = __bridge('redis', 'scan', { handle: 1 });
  const config = __bridge('redis', 'CONFIG', { handle: 1 });
  const randomkey = __bridge('redis', 'randomkey', { handle: 1 });
  return { status: 200, headers: {}, body: JSON.stringify({ scan, config, randomkey }) };
};
"#;
    let res = pool
        .execute(
            HandlerKey::new("bridge_redis_unscoped_blocked"),
            tenant_request(code, "shop_unscoped_guard"),
        )
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    for action in ["scan", "config", "randomkey"] {
        assert_eq!(
            parsed[action],
            serde_json::json!({
                "ok": false,
                "error": "redis unscoped action blocked for tenant code"
            }),
            "{action} should be rejected before native Redis dispatch"
        );
    }
}

#[tokio::test]
async fn bridge_crypto_bcrypt_verify_resolves_to_op() {
    let pool = php_pool();
    let code = r#"
globalThis.app = function(req) {
  const hash = "$2b$10$DqpfeHg1RhyMilY/GTQvgeahRja6yf5aL8dYoH6EwABQY.CZ.pnNu";
  const result = Object.fromEntries(__bridge('crypto', 'bcrypt_verify', {password: 'password123', hash}));
  return { status: 200, headers: {}, body: JSON.stringify(result) };
};
"#;
    let res = pool
        .execute(HandlerKey::new("bridge_bcrypt"), test_request(code))
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(parsed.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[tokio::test]
async fn deka_host_digest_sha256_empty_known_vector() {
    let pool = php_pool();
    let code = r#"
globalThis.app = function(req) {
  const result = __deka_host('crypto', 'digest', ['sha256', new Uint8Array()], ['crypto']);
  const data = result && result.data ? Array.from(result.data) : [];
  return { status: 200, headers: {}, body: JSON.stringify({ ok: result.ok, error: result.error, len: data.length, b0: data[0], b1: data[1] }) };
};
"#;
    let res = pool
        .execute(HandlerKey::new("deka_host_digest"), test_request(code))
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["ok"], true, "body={body}");
    assert_eq!(parsed["len"], 32);
    assert_eq!(parsed["b0"], 0xe3);
    assert_eq!(parsed["b1"], 0xb0);
}

#[tokio::test]
async fn deka_host_catalog_denies_unknown_kinds_and_actions() {
    let pool = php_pool();
    let code = r#"
globalThis.app = function(req) {
  // redis is a PHPX-only kind: absent from the injected host catalog.
  const redis = __deka_host('redis', 'get', []);
  // db is catalogued, but 'bogus' is not a db action.
  const bogus = __deka_host('db', 'bogus', []);
  return { status: 200, headers: {}, body: JSON.stringify({ redis, bogus }) };
};
"#;
    let res = pool
        .execute(
            HandlerKey::new("deka_host_deny_unknown"),
            test_request(code),
        )
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    for key in ["redis", "bogus"] {
        assert_eq!(
            parsed[key].get("ok").and_then(|v| v.as_bool()),
            Some(false),
            "{key}: unexpected envelope: {body}"
        );
        assert!(
            parsed[key]
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .contains("unknown bridge action"),
            "{key}: unexpected error: {body}"
        );
    }
}

/// RFD 27 grant gate: an explicit grants array that does not include the
/// kind denies the call with a structured HostGrantDenied envelope.
#[tokio::test]
async fn deka_host_grant_gate_denies_ungranted_kinds() {
    let pool = php_pool();
    let code = r#"
globalThis.app = function(req) {
  const denied = __deka_host('crypto', 'digest', ['sha256', new Uint8Array()], ['fs']);
  const granted = __deka_host('crypto', 'digest', ['sha256', new Uint8Array()], ['crypto']);
  return { status: 200, headers: {}, body: JSON.stringify({
    deniedOk: denied.ok,
    deniedName: denied.error && denied.error.name,
    deniedKind: denied.error && denied.error.kind,
    grantedOk: granted.ok,
    grantedLen: granted.data ? granted.data.length : 0
  }) };
};
"#;
    let res = pool
        .execute(
            HandlerKey::new("deka_host_grant_gate"),
            test_request(code),
        )
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["deniedOk"], false, "body={body}");
    assert_eq!(parsed["deniedName"], "HostGrantDenied", "body={body}");
    assert_eq!(parsed["deniedKind"], "crypto", "body={body}");
    assert_eq!(parsed["grantedOk"], true, "body={body}");
    assert_eq!(parsed["grantedLen"], 32, "body={body}");
}

#[tokio::test]
async fn deka_host_secure_compare_and_hmac() {
    let pool = php_pool();
    let code = r#"
globalThis.app = function(req) {
  const a = new Uint8Array([1, 2, 3]);
  const b = new Uint8Array([1, 2, 3]);
  const c = new Uint8Array([1, 2, 4]);
  const same = __deka_host('crypto', 'secure_compare', [a, b], ['crypto']);
  const diff = __deka_host('crypto', 'secure_compare', [a, c], ['crypto']);
  const mac = __deka_host('crypto', 'hmac', ['sha256', a, b], ['crypto']);
  return { status: 200, headers: {}, body: JSON.stringify({
    same: same.data === true,
    diff: diff.data === false,
    macOk: mac.ok === true && mac.data && mac.data.length === 32
  }) };
};
"#;
    let res = pool
        .execute(
            HandlerKey::new("deka_host_hmac_compare"),
            test_request(code),
        )
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["same"], true, "body={body}");
    assert_eq!(parsed["diff"], true, "body={body}");
    assert_eq!(parsed["macOk"], true, "body={body}");
}

/// Restores a process env var on drop. Scoped to one test; the policy set
/// through it only grants read/write under that test's own tempdir, so
/// concurrent readers of `DEKA_SECURITY_POLICY` see an equivalent net scope
/// (deka#537 was about net rules racing, which this does not touch).
struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: String) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: single-process test binary; the guard restores the
        // previous value on drop. Same pattern as deka_host's fs symlink
        // tests.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

/// deka#578: fs bridge ops dispatch through the async op. The caller gets a
/// Promise back — the old code ran the blocking op inline, stalling the
/// isolate, and a Promise result read `.ok` as undefined which silently
/// became `Err("host bridge failed")`. Awaiting the promise must resolve to
/// the same `{ok, value}` envelope as the sync path.
#[tokio::test]
async fn deka_host_fs_ops_dispatch_async_and_resolve() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path().canonicalize().expect("canonicalize tempdir");
    let target = root.join("round-trip.txt");
    let policy = serde_json::json!({
        "security": {
            "allow": {
                "read": [root.to_string_lossy()],
                "write": [root.to_string_lossy()]
            },
            "prompt": false
        }
    })
    .to_string();
    let _policy_guard = EnvGuard::set("DEKA_SECURITY_POLICY", policy);

    let path_js = serde_json::to_string(&target.to_string_lossy()).expect("json path");
    let pool = php_pool();
    let code = format!(
        r#"
globalThis.app = async function(req) {{
  const written = __deka_host('fs', 'write_file', [{path_js}, new TextEncoder().encode('hello deka#578')]);
  const writeIsPromise = written && typeof written.then === 'function';
  const w = await written;
  const pending = __deka_host('fs', 'read_file', [{path_js}]);
  const readIsPromise = pending && typeof pending.then === 'function';
  const r = await pending;
  const text = r && r.ok ? new TextDecoder().decode(r.value) : ('ERR:' + (r && r.error));
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{
    writeIsPromise: writeIsPromise,
    readIsPromise: readIsPromise,
    writeOk: !!(w && w.ok),
    readOk: !!(r && r.ok),
    text: text
  }}) }};
}};
"#
    );
    let res = pool
        .execute(HandlerKey::new("deka_host_fs_async"), test_request(&code))
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["writeIsPromise"], true, "body={body}");
    assert_eq!(parsed["readIsPromise"], true, "body={body}");
    assert_eq!(parsed["writeOk"], true, "body={body}");
    assert_eq!(parsed["readOk"], true, "body={body}");
    assert_eq!(parsed["text"], "hello deka#578", "body={body}");
}

#[tokio::test]
async fn wintertc_fetch_returns_response() {
    let pool = test_pool();
    let code = r#"
globalThis.app = {
  async fetch(request) {
    const url = request.url || "";
    return new Response("Hello World!", {
      status: 201,
      headers: { "Content-Type": "text/plain" },
    });
  },
};
"#;
    let mut req = test_request(code);
    req.request_parts = Some(RequestParts {
        url: "http://localhost/hello".to_string(),
        method: "GET".to_string(),
        headers: vec![("accept".to_string(), "text/plain".to_string())],
        body: None,
    });
    let res = pool
        .execute(HandlerKey::new("wintertc_fetch"), req)
        .await
        .expect("pool execution should succeed");
    assert!(res.success, "execution failed: {:?}", res.error);
    let result = res.result.expect("should have result");
    assert_eq!(result.get("status").and_then(|v| v.as_u64()), Some(201));
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    assert_eq!(body, "Hello World!");
    let headers = result.get("headers").expect("headers");
    let content_type = headers
        .get("content-type")
        .or_else(|| headers.get("Content-Type"))
        .and_then(|v| v.as_str());
    assert_eq!(content_type, Some("text/plain"));
}
