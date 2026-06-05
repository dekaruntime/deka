use std::sync::Arc;
use engine::{RuntimeState, RuntimeEngine, execute_request_parts};
use engine::config::RuntimeConfig;
use pool::{PoolConfig, HandlerKey};

fn test_state(handler_code: &str) -> Arc<RuntimeState> {
    let server_pool_config = PoolConfig {
        num_workers: 1,
        max_isolates_per_worker: 2,
        idle_timeout_secs: 30,
        enable_metrics: false,
        enable_code_cache: false,
        request_timeout_ms: 10_000,
        queue_timeout_ms: 10_000,
        ..PoolConfig::default()
    };
    let user_pool_config = server_pool_config.clone();
    let runtime_config = RuntimeConfig::default();
    let engine = Arc::new(RuntimeEngine::new(
        server_pool_config,
        user_pool_config,
        &runtime_config,
        Arc::new(|| vec![]),
    ));
    Arc::new(RuntimeState {
        engine,
        handler_code: handler_code.to_string(),
        handler_entry: None,
        handler_key: HandlerKey::new(format!("test_handler_{}", std::process::id())),
        dev_mode: false,
        perf_mode: false,
        perf_request_value: serde_json::Value::Null,
    })
}

#[tokio::test]
async fn request_dispatch_routes_to_correct_handler() {
    let code = r#"
globalThis.app = function(req) {
  return { status: 200, headers: {}, body: "handled by correct handler" };
};
"#;
    let state = test_state(code);
    let res = execute_request_parts(
        state,
        "http://localhost/".to_string(),
        "GET".to_string(),
        vec![],
        None,
    ).await;
    let envelope = res.expect("should return envelope");
    assert_eq!(envelope.status, 200);
    assert_eq!(envelope.body, "handled by correct handler");
}

#[tokio::test]
async fn response_status_codes_propagate_correctly() {
    let code = r#"
globalThis.app = function(req) {
  return { status: 418, headers: {"x-custom": "yes"}, body: "teapot" };
};
"#;
    let state = test_state(code);
    let envelope = execute_request_parts(
        state,
        "http://localhost/".to_string(),
        "GET".to_string(),
        vec![],
        None,
    ).await.expect("should succeed");
    assert_eq!(envelope.status, 418);
    assert_eq!(envelope.headers.get("x-custom"), Some(&"yes".to_string()));
    assert_eq!(envelope.body, "teapot");
}

#[tokio::test]
async fn not_found_returns_correct_json_error_shape() {
    let code = r#"
globalThis.app = function(req) {
  return { status: 404, headers: {"content-type": "application/json"}, body: JSON.stringify({error: "not found"}) };
};
"#;
    let state = test_state(code);
    let envelope = execute_request_parts(
        state,
        "http://localhost/missing".to_string(),
        "GET".to_string(),
        vec![],
        None,
    ).await.expect("should succeed");
    assert_eq!(envelope.status, 404);
    let parsed: serde_json::Value = serde_json::from_str(&envelope.body).unwrap();
    assert_eq!(parsed.get("error").and_then(|v| v.as_str()), Some("not found"));
}

#[tokio::test]
async fn error_envelope_shape_on_handler_panic() {
    let code = r#"
globalThis.app = function(req) {
  throw new Error("simulated handler panic");
};
"#;
    let state = test_state(code);
    let err = execute_request_parts(
        state,
        "http://localhost/".to_string(),
        "GET".to_string(),
        vec![],
        None,
    ).await.expect_err("should fail on panic");
    assert!(
        err.contains("handler execution failed"),
        "error should indicate handler execution failed: {}",
        err
    );
    assert!(
        err.contains("simulated handler panic"),
        "error should contain panic message: {}",
        err
    );
}
