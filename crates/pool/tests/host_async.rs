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
        security: ExecutionSecurity {
            policy_json,
            no_prompt: true,
        },
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
///
/// The setup is deliberately deterministic so the result does not depend on
/// runner load:
/// - Both handler isolates are warmed before the measured round, so the
///   measured latency is queue handoff + execution, not cold V8 isolate
///   creation. On a contended CI runner cold isolate boot alone can exceed
///   the latency bound, which is load noise, not the concurrency property.
/// - A marker file written *inside* the slow handler proves the slow op is
///   in flight before the fast request is submitted, so least-loaded routing
///   deterministically sends it to the sibling worker — no sleep heuristic.
/// - The overlap assertion checks the semantic property directly: the fast
///   handler began executing before the slow async op resolved.
#[cfg(unix)]
#[tokio::test]
async fn slow_async_op_does_not_block_sibling_worker_request() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fifo = dir.path().join("slow.fifo");
    mkfifo(&fifo);
    let medium_fifo = dir.path().join("medium.fifo");
    mkfifo(&medium_fifo);

    let path_js = serde_json::to_string(&fifo.to_string_lossy()).expect("json path");
    let medium_path_js = serde_json::to_string(&medium_fifo.to_string_lossy()).expect("json path");
    let marker = dir.path().join("slow_op_started.marker");
    let marker_js = serde_json::to_string(&marker.to_string_lossy()).expect("json marker");
    let medium_marker = dir.path().join("medium_started.marker");
    let medium_marker_js =
        serde_json::to_string(&medium_marker.to_string_lossy()).expect("json marker");

    // Slow handler: record that it reached the async op, then block on the
    // fifo. The marker file is how the test observes "slow op in flight".
    let slow_code = format!(
        r#"
globalThis.app = async function(req) {{
  await __deka_host('fs', 'write_file', [{marker_js}, [1]], ['fs']);
  const r = await __deka_host('fs', 'read_file', [{path_js}], ['fs']);
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{ ok: r.ok === true, tEnd: Date.now() }}) }};
}};
"#
    );
    // Medium handler: occupies its worker (marker, then ~150 ms fifo read) so
    // the fast-handler warm-up is routed to the sibling worker.
    let medium_code = format!(
        r#"
globalThis.app = async function(req) {{
  await __deka_host('fs', 'write_file', [{medium_marker_js}, [1]], ['fs']);
  await __deka_host('fs', 'read_file', [{medium_path_js}], ['fs']);
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{ medium: true }}) }};
}};
"#
    );
    let fast_code = r#"
globalThis.app = function(req) {
  const r = __deka_host('crypto', 'random_bytes', [8], ['crypto']);
  return { status: 200, headers: {}, body: JSON.stringify({ fast: true, host_ok: r.ok === true, handler_wall_ms: Date.now() }) };
};
"#;

    let pool = Arc::new(php_pool(2));
    let policy = allow_under(dir.path());

    // 1. Warm the slow handler's isolate on worker 0 (idle pool: least-loaded
    //    tie routes to the first worker) with a writer that unblocks at once.
    delayed_fifo_writer(fifo.clone(), Duration::from_millis(1), b"warm");
    let warm = pool
        .execute(
            HandlerKey::new("async_slow_worker_a"),
            request_with_security(&slow_code, policy.clone()),
        )
        .await
        .expect("warm execution");
    assert!(warm.success, "warm request failed: {:?}", warm.error);
    assert!(marker.exists(), "warm round must write the marker");

    // 2. Occupy worker 0 with the medium request; its marker proves the
    //    request is active, so the fast-handler warm-up routes to worker 1.
    delayed_fifo_writer(medium_fifo.clone(), Duration::from_millis(150), b"medium");
    let medium_pool = Arc::clone(&pool);
    let medium_policy = policy.clone();
    let medium = tokio::spawn(async move {
        medium_pool
            .execute(
                HandlerKey::new("async_medium_occupier"),
                request_with_security(&medium_code, medium_policy),
            )
            .await
            .expect("medium execution")
    });
    wait_for_file(&medium_marker, Duration::from_secs(10)).await;

    // 3. Warm the fast handler's isolate — worker 0 is busy, so this routes
    //    to worker 1 and leaves a warm isolate there for the measured round.
    let fast_warm = pool
        .execute(
            HandlerKey::new("async_fast_worker_b"),
            request_with_security(fast_code, policy.clone()),
        )
        .await
        .expect("fast warm execution");
    assert!(
        fast_warm.success,
        "fast warm request failed: {:?}",
        fast_warm.error
    );

    // 4. Let the occupier finish; both workers are idle again.
    let medium = medium.await.expect("medium task");
    assert!(
        medium.success,
        "medium request failed: {:?}",
        medium.error
    );

    // 5. Measured round: reset the marker, then block the slow handler on a
    //    fresh 400 ms writer. The marker reappearing proves the slow op is in
    //    flight on worker 0 before the fast request is submitted.
    std::fs::remove_file(&marker).expect("reset marker");
    delayed_fifo_writer(fifo.clone(), Duration::from_millis(400), b"slow");
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
    wait_for_file(&marker, Duration::from_secs(10)).await;

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
    let fast_handler_wall = fast
        .result
        .as_ref()
        .and_then(|r| r.get("body"))
        .and_then(|b| serde_json::from_str::<serde_json::Value>(b.as_str().unwrap_or("")).ok())
        .and_then(|v| v.get("handler_wall_ms").and_then(|x| x.as_u64()))
        .expect("fast handler wall clock");
    let slow_t_end = slow
        .result
        .as_ref()
        .and_then(|r| r.get("body"))
        .and_then(|b| serde_json::from_str::<serde_json::Value>(b.as_str().unwrap_or("")).ok())
        .and_then(|v| v.get("tEnd").and_then(|x| x.as_u64()))
        .expect("slow op end wall clock");
    let fast_host_ok = fast
        .result
        .as_ref()
        .and_then(|r| r.get("body"))
        .and_then(|b| serde_json::from_str::<serde_json::Value>(b.as_str().unwrap_or("")).ok())
        .and_then(|v| v.get("host_ok").and_then(|x| x.as_bool()))
        .expect("fast host op result");
    assert!(
        fast_host_ok,
        "fast request's own host op did not complete cleanly"
    );
    assert!(
        fast_handler_wall < slow_t_end,
        "fast handler started after the slow op resolved (serialized): fast={fast_handler_wall} slow_end={slow_t_end}"
    );
}

/// Poll until `path` exists (the handlers signal in-flight state through
/// marker files), failing after `timeout`.
#[cfg(unix)]
async fn wait_for_file(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
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
                security: ExecutionSecurity {
                    policy_json: deny_read_policy(),
                    no_prompt: true,
                },
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
/// the Promise resolves to the typed `FsError.PermissionDenied`, never an
/// empty successful byte payload, and the request itself succeeds.
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
  const permission = e && e.value ? e.value : null;
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{
    threw,
    ok: r ? r.ok : null,
    hasValue: !!(r && Object.prototype.hasOwnProperty.call(r, 'value')),
    enumName: e && e.__enum,
    caseName: e && e.__case,
    capability: permission && permission.capability,
    target: permission && permission.target
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
    assert_eq!(parsed["hasValue"], false, "body={parsed}");
    assert_eq!(parsed["enumName"], "FsError", "body={parsed}");
    assert_eq!(parsed["caseName"], "PermissionDenied", "body={parsed}");
    assert_eq!(parsed["capability"], "read", "body={parsed}");
    assert!(
        parsed["target"]
            .as_str()
            .is_some_and(|target| target.ends_with("secret.txt")),
        "target should name the denied path: {parsed}"
    );
}

/// The public `_sync` actions use the host's synchronous call path. They are
/// not a Promise disguised as a synchronous wrapper, and bytes remain exact.
#[tokio::test]
async fn fs_sync_actions_are_blocking_and_preserve_strict_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("source.bin");
    std::fs::write(&source, [0, 0xff, 0x80, b'A']).expect("write binary fixture");

    let source_js = serde_json::to_string(&source.to_string_lossy()).expect("json path");
    let output_js = serde_json::to_string(&dir.path().join("out.bin").to_string_lossy())
        .expect("json output path");
    let code = format!(
        r#"
globalThis.app = function(req) {{
  const read = __deka_host('fs', 'read_file_sync', [{source_js}], ['fs']);
  const bad = __deka_host('fs', 'write_file_sync', [{output_js}, 'not bytes'], ['fs']);
  return {{ status: 200, headers: {{}}, body: JSON.stringify({{
    readIsPromise: !!(read && typeof read.then === 'function'),
    readOk: read && read.ok,
    payload: read && read.value ? Array.from(read.value) : null,
    invalidEnum: bad && bad.error && bad.error.__enum,
    invalidCase: bad && bad.error && bad.error.__case
  }}) }};
}};
"#
    );

    let pool = php_pool(1);
    let response = pool
        .execute(
            HandlerKey::new("fs_sync_strict_bytes"),
            request_with_security(&code, allow_under(dir.path())),
        )
        .await
        .expect("pool execution");
    let parsed: serde_json::Value =
        serde_json::from_str(&body_of(&response)).expect("json body");

    assert_eq!(parsed["readIsPromise"], false, "body={parsed}");
    assert_eq!(parsed["readOk"], true, "body={parsed}");
    assert_eq!(parsed["payload"], serde_json::json!([0, 255, 128, 65]), "body={parsed}");
    assert_eq!(parsed["invalidEnum"], "FsError", "body={parsed}");
    assert_eq!(parsed["invalidCase"], "InvalidPayload", "body={parsed}");
    assert!(
        !dir.path().join("out.bin").exists(),
        "a non-bytes payload must never be converted and written"
    );
}
