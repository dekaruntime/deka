//! RFD 27 bridge sync/async behavior tests (deka#755 chunk D).
//!
//! Pins the dsc 0.8.1 emit contract on the wire:
//! - exactly `fs.{read_file,write_file,read_dir,mkdirs}` are async — the
//!   returned Promise never rejects; failures (including permission denials)
//!   resolve to the `{ok:false, error}` envelope.
//! - every other action is sync — `__deka_host` returns the envelope itself,
//!   never a Promise.
//!
//! These tests are inline-handler based (no dsc, no on-disk project): the
//! catalog/grant behavior under test lives in the pool bootstrap.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pool::{
    ExecutionMode, ExecutionSecurity, HandlerKey, IsolatePool, PoolConfig, RequestData,
    RequestParts,
};

fn php_pool(num_workers: usize) -> IsolatePool {
    let config = PoolConfig {
        num_workers,
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

fn request_with_security(handler_code: &str, policy_json: String) -> RequestData {
    RequestData {
        handler_code: handler_code.to_string(),
        handler_entry: None,
        module_root: None,
        request_value: serde_json::Value::Null,
        request_parts: Some(RequestParts {
            url: "http://localhost/".to_string(),
            method: "GET".to_string(),
            headers: Vec::new(),
            body: None,
        }),
        mode: ExecutionMode::Request,
        security: Some(ExecutionSecurity {
            policy_json,
            no_prompt: true,
        }),
    }
}

/// Allow read/write exactly under `root` for this request only (deka#725
/// per-execution context — no process-env races).
fn allow_under(root: &Path) -> String {
    serde_json::json!({
        "security": {
            "allow": {
                "read": [root.to_string_lossy()],
                "write": [root.to_string_lossy()]
            },
            "prompt": false
        }
    })
    .to_string()
}

fn body_of(response: &pool::IsolateResponse) -> String {
    assert!(
        response.success,
        "execution failed: {:?}",
        response.error
    );
    response
        .result
        .as_ref()
        .expect("response result")
        .get("body")
        .and_then(serde_json::Value::as_str)
        .expect("response body")
        .to_string()
}

#[cfg(unix)]
fn mkfifo(path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path = CString::new(path.as_os_str().as_bytes()).expect("fifo path cstring");
    let result = unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) };
    assert_eq!(result, 0, "mkfifo {} failed", path.display());
}

/// Spawn a writer that opens the fifo (blocks until the reader arrives),
/// sleeps, then writes and closes.
#[cfg(unix)]
fn delayed_fifo_writer(path: PathBuf, delay: Duration, payload: &'static [u8]) {
    std::thread::spawn(move || {
        use std::io::Write;
        let mut file = std::fs::File::create(&path).expect("open fifo for write");
        std::thread::sleep(delay);
        file.write_all(payload).expect("write fifo payload");
    });
}

/// Case B1 — a deliberately delayed async op genuinely yields: two concurrent
/// `fs.read_file` calls on fifos whose writers both unblock after ~400 ms
/// complete in ~400 ms of handler-observed time. An isolate that ran the ops
/// inline (the deka#578 bug) would serialize them: ~800 ms.
#[cfg(unix)]
#[tokio::test]
async fn delayed_async_fs_reads_run_concurrently_not_inline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fifo_a = dir.path().join("a.fifo");
    let fifo_b = dir.path().join("b.fifo");
    mkfifo(&fifo_a);
    mkfifo(&fifo_b);
    delayed_fifo_writer(fifo_a.clone(), Duration::from_millis(400), b"alpha");
    delayed_fifo_writer(fifo_b.clone(), Duration::from_millis(400), b"beta");

    let path_a = serde_json::to_string(&fifo_a.to_string_lossy()).expect("json path");
    let path_b = serde_json::to_string(&fifo_b.to_string_lossy()).expect("json path");
    let code = format!(
        r#"
globalThis.app = async function(req) {{
  const t0 = Date.now();
  const [a, b] = await Promise.all([
    __deka_host('fs', 'read_file', [{path_a}], ['fs']),
    __deka_host('fs', 'read_file', [{path_b}], ['fs'])
  ]);
  const elapsed = Date.now() - t0;
  const text = (r) => r && r.ok ? new TextDecoder().decode(r.value) : ('ERR:' + JSON.stringify(r && r.error));
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{
    ok: a.ok === true && b.ok === true,
    textA: text(a), textB: text(b), elapsed
  }}) }};
}};
"#
    );

    let pool = php_pool(1);
    let started = Instant::now();
    let response = pool
        .execute(
            HandlerKey::new("async_fs_concurrent_reads"),
            request_with_security(&code, allow_under(dir.path())),
        )
        .await
        .expect("pool execution");
    let parsed: serde_json::Value =
        serde_json::from_str(&body_of(&response)).expect("json body");

    assert_eq!(parsed["ok"], serde_json::json!(true), "body={parsed}");
    assert_eq!(parsed["textA"], "alpha", "body={parsed}");
    assert_eq!(parsed["textB"], "beta", "body={parsed}");
    // Both writers unblock at ~400 ms; serialized inline execution would need
    // >= 800 ms. Generous bound keeps this stable on slow CI.
    let elapsed = parsed["elapsed"].as_u64().expect("elapsed ms");
    assert!(
        elapsed < 750,
        "fs reads appear serialized (isolate did not yield): {elapsed}ms"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "request wall time blew up: {:?}",
        started.elapsed()
    );
}

/// Case B1 (pool level) — while one worker's isolate awaits a slow async op,
/// a sibling worker serves an immediate request; the immediate one completes
/// first. (Within one worker, requests are processed sequentially by design,
/// so the two-request comparison needs two workers.)
#[cfg(unix)]
#[tokio::test]
async fn slow_async_op_does_not_block_sibling_worker_request() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fifo = dir.path().join("slow.fifo");
    mkfifo(&fifo);
    delayed_fifo_writer(fifo.clone(), Duration::from_millis(400), b"slow");

    let path_js = serde_json::to_string(&fifo.to_string_lossy()).expect("json path");
    let slow_code = format!(
        r#"
globalThis.app = async function(req) {{
  const r = await __deka_host('fs', 'read_file', [{path_js}], ['fs']);
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{ ok: r.ok === true }}) }};
}};
"#
    );
    let fast_code = r#"
globalThis.app = function(req) {
  return { status: 200, headers: {}, body: JSON.stringify({ fast: true }) };
};
"#;

    let pool = Arc::new(php_pool(2));
    let policy = allow_under(dir.path());

    let slow_pool = Arc::clone(&pool);
    let slow_policy = policy.clone();
    let slow = tokio::spawn(async move {
        slow_pool
            .execute(
                HandlerKey::new("async_slow_worker_a"),
                request_with_security(&slow_code, slow_policy),
            )
            .await
            .expect("slow execution")
    });
    // Give the slow request time to become active so least-loaded routing
    // sends the fast one to the sibling worker.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let fast_started = Instant::now();
    let fast = pool
        .execute(
            HandlerKey::new("async_fast_worker_b"),
            request_with_security(fast_code, policy),
        )
        .await
        .expect("fast execution");
    let fast_elapsed = fast_started.elapsed();

    assert!(fast.success, "fast request failed: {:?}", fast.error);
    assert!(
        fast_elapsed < Duration::from_millis(300),
        "fast request waited behind the slow async op: {fast_elapsed:?}"
    );
    let slow = slow.await.expect("slow task");
    assert!(slow.success, "slow request failed: {:?}", slow.error);
}

/// Case B2 — sync ops never return a Promise: the envelope comes back
/// synchronously with the bytes as a Uint8Array (never Array<number>).
#[tokio::test]
async fn sync_bridge_op_returns_envelope_not_promise() {
    let pool = php_pool(1);
    let code = r#"
globalThis.app = function(req) {
  const r = __deka_host('crypto', 'random_bytes', [8], ['crypto']);
  return { status: 200, headers: {}, body: JSON.stringify({
    isPromise: r instanceof Promise,
    hasThen: r !== null && typeof r === 'object' && typeof r.then === 'function',
    ok: r.ok === true,
    isUint8Array: r.value instanceof Uint8Array,
    isNumberArray: Array.isArray(r.value),
    len: r.value && r.value.length
  }) };
};
"#;
    let response = pool
        .execute(
            HandlerKey::new("sync_not_promise"),
            RequestData {
                handler_code: code.to_string(),
                handler_entry: None,
                module_root: None,
                request_value: serde_json::Value::Null,
                request_parts: None,
                mode: ExecutionMode::Request,
                security: None,
            },
        )
        .await
        .expect("pool execution");
    let parsed: serde_json::Value =
        serde_json::from_str(&body_of(&response)).expect("json body");
    assert_eq!(parsed["isPromise"], serde_json::json!(false), "body={parsed}");
    assert_eq!(parsed["hasThen"], serde_json::json!(false), "body={parsed}");
    assert_eq!(parsed["ok"], serde_json::json!(true), "body={parsed}");
    assert_eq!(parsed["isUint8Array"], serde_json::json!(true), "body={parsed}");
    assert_eq!(parsed["isNumberArray"], serde_json::json!(false), "body={parsed}");
    assert_eq!(parsed["len"], serde_json::json!(8), "body={parsed}");
}

fn deny_read_policy() -> String {
    serde_json::json!({
        "security": {
            "allow": { "read": ["/nonexistent-deka-deny-read-canary"] },
            "prompt": false
        }
    })
    .to_string()
}

/// Case B3 — an async permission denial never crosses as a throw/rejection:
/// the Promise resolves to `{ok:false, error:{name:"PermissionDenied",
/// capability, target}}` and the request itself succeeds.
#[tokio::test]
async fn async_bridge_rejection_never_crosses_as_throw() {
    let dir = tempfile::tempdir().expect("tempdir");
    let secret = dir.path().join("secret.txt");
    std::fs::write(&secret, "top secret").expect("write fixture");

    let path_js = serde_json::to_string(&secret.to_string_lossy()).expect("json path");
    let code = format!(
        r#"
globalThis.app = async function(req) {{
  let threw = null;
  let r;
  try {{
    r = await __deka_host('fs', 'read_file', [{path_js}], ['fs']);
  }} catch (err) {{
    threw = String(err && err.message ? err.message : err);
  }}
  const e = r && r.error ? r.error : null;
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{
    threw,
    ok: r ? r.ok : null,
    name: e && e.name,
    capability: e && e.capability,
    target: e && e.target
  }}) }};
}};
"#
    );

    // Per-execution policy (RequestData.security) — the fs async op captures
    // the isolate thread's security context and re-installs it on the
    // blocking-pool thread, so the deny-read policy is enforced there too.
    let pool = php_pool(1);
    let response = pool
        .execute(
            HandlerKey::new("async_denial_envelope"),
            request_with_security(&code, deny_read_policy()),
        )
        .await
        .expect("pool execution");
    let parsed: serde_json::Value =
        serde_json::from_str(&body_of(&response)).expect("json body");

    assert_eq!(parsed["threw"], serde_json::Value::Null, "body={parsed}");
    assert_eq!(parsed["ok"], serde_json::json!(false), "body={parsed}");
    assert_eq!(parsed["name"], "PermissionDenied", "body={parsed}");
    assert_eq!(parsed["capability"], "read", "body={parsed}");
    assert!(
        parsed["target"]
            .as_str()
            .is_some_and(|target| target.ends_with("secret.txt")),
        "target should name the denied path: {parsed}"
    );
}

