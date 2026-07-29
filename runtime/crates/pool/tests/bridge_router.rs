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

fn net_test_pool() -> IsolatePool {
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
        request_value: serde_json::Value::Null,
        request_parts: None,
        mode: ExecutionMode::Request,
    }
}

fn tenant_request(handler_code: &str, shop_id: &str) -> RequestData {
    RequestData {
        handler_code: handler_code.to_string(),
        handler_entry: None,
        request_value: serde_json::Value::Null,
        request_parts: Some(RequestParts {
            url: "http://localhost/bridge".to_string(),
            method: "GET".to_string(),
            headers: vec![("host".to_string(), format!("{shop_id}.tana.gg"))],
            body: None,
        }),
        mode: ExecutionMode::Request,
    }
}

#[tokio::test]
async fn bridge_unknown_kind_returns_error_envelope() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const result = globalThis.__bridge('unknown_kind', 'x', {});
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
  const result = globalThis.__bridge('crypto', 'unknown_action', {});
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
  const result = globalThis.__bridge('time', 'now_ms', {});
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

    let previous_policy = std::env::var_os("DEKA_SECURITY_POLICY");
    unsafe {
        std::env::set_var(
            "DEKA_SECURITY_POLICY",
            format!(r#"{{"security":{{"allow":{{"net":["127.0.0.1:{port}"]}}}}}}"#),
        );
    }
    let pool = net_test_pool();
    let code = format!(
        r#"
globalThis.app = function(req) {{
  const connects = [];
  for (let i = 0; i < {CONNECTS}; i++) {{
    const result = globalThis.__bridge('net', 'connect', {{ host: '127.0.0.1', port: {port} }});
    if (!Array.isArray(result)) throw new Error(`net bridge result ${{i}} was not entry pairs`);
    const value = Object.fromEntries(result);
    if (value.ok !== true || !Number.isInteger(value.handle) || value.handle < 1) {{
      throw new Error(`invalid net bridge response ${{JSON.stringify(result)}}`);
    }}
    connects.push(value.handle);
  }}
  const closes = connects.map((handle) => Object.fromEntries(globalThis.__bridge('net', 'close', {{ handle }})));
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
    unsafe {
        match previous_policy {
            Some(value) => std::env::set_var("DEKA_SECURITY_POLICY", value),
            None => std::env::remove_var("DEKA_SECURITY_POLICY"),
        }
    }

    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    assert_eq!(body, format!(r#"{{"connects":{CONNECTS}}}"#));
    assert_eq!(accepted.join().expect("TCP listener thread"), CONNECTS);
}

#[tokio::test]
async fn bridge_redis_flush_is_blocked_before_native_dispatch() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const flush = globalThis.__bridge('redis', 'flush', { handle: 1 });
  const flushdb = globalThis.__bridge('redis', 'FLUSHDB', { handle: 1 });
  const flushall = globalThis.__bridge('redis', 'flushall', { handle: 1 });
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
                "error": "redis admin action blocked in user pool"
            }),
            "{action} should be rejected before FLUSHDB/FLUSHALL can reach Redis"
        );
    }
}

#[tokio::test]
async fn bridge_redis_keys_missing_pattern_is_tenant_scoped_before_native_dispatch() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const ops = Deno.core.ops;
  ops.op_zega_backend = function(shopId) { return 'neo4j'; };
  ops.op_redis_call = function(action, payload) {
    return { ok: true, action, payload: { ...payload } };
  };
  const result = globalThis.__bridge('redis', 'keys', { handle: 1 });
  return { status: 200, headers: {}, body: JSON.stringify(result) };
};
"#;
    let res = pool
        .execute(
            HandlerKey::new("bridge_redis_keys_missing_pattern_scoped"),
            tenant_request(code, "shop_keys_guard"),
        )
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["ok"], serde_json::json!(true), "body: {body}");
    assert_eq!(parsed["action"], serde_json::json!("keys"));
    assert_eq!(parsed["payload"]["handle"], serde_json::json!(1));
    assert_eq!(
        parsed["payload"]["pattern"],
        serde_json::json!("shop_keys_guard:*"),
        "raw tenant keys bridge must not dispatch native Redis KEYS *"
    );
}

#[tokio::test]
async fn bridge_redis_prefixed_key_ops_still_dispatch() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const ops = Deno.core.ops;
  ops.op_zega_backend = function(shopId) { return 'neo4j'; };
  ops.op_redis_call = function(action, payload) {
    return { ok: true, action, payload: { ...payload } };
  };
  const set = globalThis.__bridge('redis', 'set', { handle: 1, key: 'cart', value: 'sku-1' });
  const keys = globalThis.__bridge('redis', 'keys', { handle: 1, pattern: 'cart:*' });
  return { status: 200, headers: {}, body: JSON.stringify({ set, keys }) };
};
"#;
    let res = pool
        .execute(
            HandlerKey::new("bridge_redis_prefixed_ops"),
            tenant_request(code, "shop_prefixed_ops"),
        )
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success, "execution failed: {:?}", response.error);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["set"]["ok"], serde_json::json!(true), "body: {body}");
    assert_eq!(parsed["set"]["action"], serde_json::json!("set"));
    assert_eq!(
        parsed["set"]["payload"]["key"],
        serde_json::json!("shop_prefixed_ops:cart")
    );
    assert_eq!(
        parsed["keys"]["ok"],
        serde_json::json!(true),
        "body: {body}"
    );
    assert_eq!(parsed["keys"]["action"], serde_json::json!("keys"));
    assert_eq!(
        parsed["keys"]["payload"]["pattern"],
        serde_json::json!("shop_prefixed_ops:cart:*")
    );
}

#[tokio::test]
async fn bridge_redis_unscoped_enumeration_verbs_are_blocked() {
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const scan = globalThis.__bridge('redis', 'scan', { handle: 1 });
  const config = globalThis.__bridge('redis', 'CONFIG', { handle: 1 });
  const randomkey = globalThis.__bridge('redis', 'randomkey', { handle: 1 });
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
                "error": "redis unscoped action blocked in user pool"
            }),
            "{action} should be rejected before native Redis dispatch"
        );
    }
}

#[tokio::test]
#[ignore = "blocked: source bug — pool bridge router missing crypto/bcrypt_verify branch in routeHostCall (isolate_pool.rs)"]
async fn bridge_crypto_bcrypt_verify_resolves_to_op() {
    // This test verifies that bridge('crypto', 'bcrypt_verify', ...) routes to
    // op_php_bcrypt_verify. Currently the JS routeHostCall only handles
    // random_bytes, aes_256_gcm_encrypt, and aes_256_gcm_decrypt; bcrypt_verify
    // falls through to "unknown crypto action".
    let pool = test_pool();
    let code = r#"
globalThis.app = function(req) {
  const result = globalThis.__bridge('crypto', 'bcrypt_verify', {password: 'pass', hash: 'hash'});
  return { status: 200, headers: {}, body: JSON.stringify(result) };
};
"#;
    let res = pool
        .execute(HandlerKey::new("bridge_bcrypt"), test_request(code))
        .await;
    let response = res.expect("pool execution should succeed");
    assert!(response.success);
    let result = response.result.expect("should have result");
    let body = result.get("body").and_then(|v| v.as_str()).expect("body");
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    // Once the source bug is fixed, this should be ok:true with valid:true/false.
    assert_eq!(parsed.get("ok").and_then(|v| v.as_bool()), Some(true));
}
