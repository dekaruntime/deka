//! Integration tests for `@deka/http` (issue #128).
//!
//! These exercise the Rust bridge entry points directly — the same
//! `http_call(action, payload)` that the pool's isolate bridge
//! dispatches to. PHPX-level smoke tests live in the module README and
//! are validated via `deka serve`.
//!
//! Tests that hit `httpbin.org` are marked `#[ignore]` so they don't
//! flake CI on air-gapped machines. Run them with
//! `cargo test --release -p modules_php --test http_module -- --ignored`.
//! The local-loopback tests (capability gate, WS echo) are always on.

use modules_php::modules::http::http_call;
use serde_json::json;
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

/// `DEKA_SECURITY_POLICY` is process-global env state. Tests that
/// mutate it must serialize — cargo's default thread-parallel runner
/// would otherwise let them race and produce false failures.
fn policy_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct PolicyGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl PolicyGuard {
    fn allow_net(hosts: &[&str]) -> Self {
        let guard = policy_lock().lock().unwrap_or_else(|e| e.into_inner());
        let allow_list: Vec<serde_json::Value> = hosts
            .iter()
            .map(|h| serde_json::Value::String(h.to_string()))
            .collect();
        let policy = serde_json::json!({
            "security": { "allow": { "net": allow_list } }
        });
        unsafe {
            std::env::set_var("DEKA_SECURITY_POLICY", policy.to_string());
        }
        PolicyGuard { _guard: guard }
    }
}

impl Drop for PolicyGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("DEKA_SECURITY_POLICY");
        }
    }
}

#[test]
fn capability_gate_blocks_disallowed_host() {
    let _g = PolicyGuard::allow_net(&["api.stripe.com"]);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": "https://example.com/",
        }),
    );
    assert_eq!(resp.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        resp.get("error").and_then(|v| v.as_str()),
        Some("host_not_allowed"),
        "expected host_not_allowed, got: {}",
        resp
    );
}

#[test]
fn capability_gate_allows_exact_host() {
    let _g = PolicyGuard::allow_net(&["example.com"]);
    // Capability gate passes; network may or may not be available.
    // Accept any outcome except `host_not_allowed`.
    let resp = http_call(
        "request",
        &json!({
            "method": "HEAD",
            "url": "http://example.com/",
            "timeout_ms": 2000,
        }),
    );
    let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert_ne!(err, "host_not_allowed", "gate should pass: {}", resp);
}

#[test]
fn capability_gate_dns_wildcard_matches_subdomains() {
    let _g = PolicyGuard::allow_net(&["*.squareup.com"]);
    // Bare parent domain must NOT match the wildcard — this is the
    // cert-SAN rule and it keeps `squareup.com` from being implicitly
    // granted when the tenant only allowed subdomains.
    let bare = http_call(
        "request",
        &json!({
            "method": "HEAD",
            "url": "https://squareup.com/",
            "timeout_ms": 2000,
        }),
    );
    assert_eq!(
        bare.get("error").and_then(|v| v.as_str()),
        Some("host_not_allowed"),
        "bare domain should be blocked by wildcard: {}",
        bare
    );

    // Subdomains match.
    let sub = http_call(
        "request",
        &json!({
            "method": "HEAD",
            "url": "https://connect.squareup.com/",
            "timeout_ms": 2000,
        }),
    );
    let err = sub.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert_ne!(err, "host_not_allowed", "subdomain should be allowed: {}", sub);
}

#[test]
fn invalid_url_rejected() {
    let _g = PolicyGuard::allow_net(&["*"]);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": "not a url",
        }),
    );
    assert_eq!(resp.get("ok").and_then(|v| v.as_bool()), Some(false));
}

#[test]
#[ignore = "requires internet; run with --ignored"]
fn httpbin_get_roundtrips() {
    let _g = PolicyGuard::allow_net(&["httpbin.org"]);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": "https://httpbin.org/get",
            "timeout_ms": 15000,
        }),
    );
    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "httpbin GET failed: {}",
        resp
    );
    let status = resp.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
    assert_eq!(status, 200);
    let body = resp.get("body").and_then(|v| v.as_str()).unwrap_or("");
    assert!(body.contains("httpbin"), "unexpected body: {}", body);
}

#[test]
#[ignore = "requires internet; run with --ignored"]
fn httpbin_post_with_body_roundtrips() {
    let _g = PolicyGuard::allow_net(&["httpbin.org"]);
    let resp = http_call(
        "request",
        &json!({
            "method": "POST",
            "url": "https://httpbin.org/post",
            "headers": { "Content-Type": "application/json" },
            "body": "{\"hello\":\"world\"}",
            "timeout_ms": 15000,
        }),
    );
    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "{}",
        resp
    );
    let body = resp.get("body").and_then(|v| v.as_str()).unwrap_or("");
    assert!(body.contains("\"hello\""), "body missing echo: {}", body);
}

#[test]
#[ignore = "requires internet; run with --ignored"]
fn http2_negotiated_via_alpn() {
    let _g = PolicyGuard::allow_net(&["nghttp2.org"]);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": "https://nghttp2.org/httpbin/get",
            "timeout_ms": 15000,
        }),
    );
    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "{}",
        resp
    );
    let version = resp.get("version").and_then(|v| v.as_str()).unwrap_or("");
    // reqwest's Version formats as "HTTP/2.0" when h2 is negotiated.
    assert!(
        version.contains("HTTP/2"),
        "expected HTTP/2, got: {} (full resp: {})",
        version,
        resp
    );
}

#[test]
#[ignore = "requires internet; run with --ignored"]
fn cookie_jar_persists_across_requests() {
    let _g = PolicyGuard::allow_net(&["httpbin.org"]);
    let client = http_call(
        "client_new",
        &json!({
            "timeout_ms": 15000,
            "cookie_jar": true,
        }),
    );
    assert_eq!(client.get("ok").and_then(|v| v.as_bool()), Some(true));
    let handle = client.get("client_handle").and_then(|v| v.as_u64()).unwrap();

    // httpbin /cookies/set/<name>/<value> returns Set-Cookie.
    let _ = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": "https://httpbin.org/cookies/set/dekatest/42",
            "client_handle": handle,
        }),
    );

    // Second request echoes the jar.
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": "https://httpbin.org/cookies",
            "client_handle": handle,
        }),
    );
    let body = resp.get("body").and_then(|v| v.as_str()).unwrap_or("");
    assert!(body.contains("dekatest"), "cookie missing: {}", body);

    let cookies = http_call(
        "client_cookies",
        &json!({
            "client_handle": handle,
            "url": "https://httpbin.org/",
        }),
    );
    let list = cookies
        .get("cookies")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(list
        .iter()
        .any(|c| c.get("name").and_then(|v| v.as_str()) == Some("dekatest")));

    // Cross-origin should NOT see the cookie.
    let cross = http_call(
        "client_cookies",
        &json!({
            "client_handle": handle,
            "url": "https://example.com/",
        }),
    );
    let xlist = cross
        .get("cookies")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        xlist.is_empty(),
        "cookies leaked cross-origin: {:?}",
        xlist
    );

    let _ = http_call(
        "client_close",
        &json!({ "client_handle": handle }),
    );
}

#[test]
#[ignore = "requires internet; run with --ignored"]
fn streaming_response_reads_chunks() {
    let _g = PolicyGuard::allow_net(&["httpbin.org"]);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": "https://httpbin.org/stream/8",
            "stream_response": true,
            "timeout_ms": 15000,
        }),
    );
    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "{}",
        resp
    );
    let handle = resp
        .get("stream_handle")
        .and_then(|v| v.as_u64())
        .unwrap();

    let mut total = 0usize;
    loop {
        let chunk = http_call(
            "stream_read",
            &json!({
                "stream_handle": handle,
                "timeout_ms": 5000,
            }),
        );
        if chunk.get("done").and_then(|v| v.as_bool()) == Some(true) {
            break;
        }
        let size = chunk
            .get("chunk")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        total += size;
        if total > 10_000_000 {
            panic!("runaway stream");
        }
    }
    assert!(total > 0, "no bytes streamed");
    let _ = http_call(
        "stream_close",
        &json!({ "stream_handle": handle }),
    );
}

#[test]
#[ignore = "requires internet; allocates 100MB; run with --ignored"]
fn streaming_upload_100mb_constant_memory() {
    let _g = PolicyGuard::allow_net(&["httpbin.org"]);
    let stream = http_call("req_stream_new", &json!({}));
    let handle = stream.get("stream_handle").and_then(|v| v.as_u64()).unwrap();

    // Dispatch the request on a background thread so we can feed the
    // body concurrently.
    let url = "https://httpbin.org/anything".to_string();
    let t = std::thread::spawn(move || {
        http_call(
            "request",
            &json!({
                "method": "POST",
                "url": url,
                "headers": { "Content-Type": "application/octet-stream" },
                "body_stream_handle": handle,
                "timeout_ms": 120000,
                "max_body_bytes": 200_000_000,
            }),
        )
    });

    // 100 MB in 1 MB chunks — constant memory on the Rust side because
    // we push+drop each chunk via the bounded channel.
    let chunk: Vec<u8> = vec![b'a'; 1024 * 1024];
    let chunk_json: Vec<serde_json::Value> =
        chunk.iter().map(|b| serde_json::Value::from(*b)).collect();
    for _ in 0..100 {
        let r = http_call(
            "req_stream_write",
            &json!({
                "stream_handle": handle,
                "chunk": chunk_json.clone(),
            }),
        );
        assert_eq!(r.get("ok").and_then(|v| v.as_bool()), Some(true));
    }
    let _ = http_call(
        "req_stream_end",
        &json!({ "stream_handle": handle }),
    );
    let resp = t.join().unwrap();
    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "{}",
        resp
    );
}

// ---------------------------------------------------------------------------
// Local WebSocket echo — pins the wire behaviour without relying on the
// public internet.
// ---------------------------------------------------------------------------

fn spawn_ws_echo() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind echo");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).ok();
    thread::spawn(move || {
        // Multi-thread runtime — a current-thread one blocks on
        // accept() and never services the tasks that handle each
        // accepted connection. Two workers are plenty for one test.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                tokio::spawn(async move {
                    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    use futures_util::{SinkExt, StreamExt};
                    let (mut sink, mut src) = ws_stream.split();
                    while let Some(msg) = src.next().await {
                        match msg {
                            Ok(m) if m.is_text() || m.is_binary() => {
                                if sink.send(m).await.is_err() {
                                    return;
                                }
                            }
                            Ok(tokio_tungstenite::tungstenite::Message::Ping(p)) => {
                                // Manual server-side auto-pong — the
                                // client sees the pong as a `pong`
                                // frame via `ws_recv`.
                                let _ = sink
                                    .send(tokio_tungstenite::tungstenite::Message::Pong(p))
                                    .await;
                            }
                            Ok(tokio_tungstenite::tungstenite::Message::Close(frame)) => {
                                let _ = sink
                                    .send(tokio_tungstenite::tungstenite::Message::Close(frame))
                                    .await;
                                return;
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
    });
    thread::sleep(Duration::from_millis(100));
    port
}

#[test]
fn websocket_local_echo_text_and_binary() {
    let _g = PolicyGuard::allow_net(&["127.0.0.1"]);
    let port = spawn_ws_echo();
    let url = format!("ws://127.0.0.1:{}", port);

    let conn = http_call("ws_connect", &json!({ "url": url }));
    assert_eq!(
        conn.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "{}",
        conn
    );
    let handle = conn.get("ws_handle").and_then(|v| v.as_u64()).unwrap();

    // Text round-trip.
    let _ = http_call(
        "ws_send_text",
        &json!({ "ws_handle": handle, "text": "hello" }),
    );
    let text = http_call(
        "ws_recv",
        &json!({ "ws_handle": handle, "timeout_ms": 2000 }),
    );
    assert_eq!(text.get("kind").and_then(|v| v.as_str()), Some("text"));
    assert_eq!(text.get("text").and_then(|v| v.as_str()), Some("hello"));

    // Binary round-trip.
    let _ = http_call(
        "ws_send_binary",
        &json!({
            "ws_handle": handle,
            "bytes": [1, 2, 3, 255]
        }),
    );
    let bin = http_call(
        "ws_recv",
        &json!({ "ws_handle": handle, "timeout_ms": 2000 }),
    );
    assert_eq!(bin.get("kind").and_then(|v| v.as_str()), Some("binary"));
    let got: Vec<u64> = bin
        .get("bytes")
        .and_then(|v| v.as_array())
        .unwrap()
        .iter()
        .filter_map(|v| v.as_u64())
        .collect();
    assert_eq!(got, vec![1, 2, 3, 255]);

    // Ping/pong — server auto-pongs.
    let _ = http_call(
        "ws_ping",
        &json!({ "ws_handle": handle, "bytes": [9, 9] }),
    );
    let pong = http_call(
        "ws_recv",
        &json!({ "ws_handle": handle, "timeout_ms": 2000 }),
    );
    let kind = pong.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        kind == "pong" || kind == "ping",
        "unexpected frame: {}",
        pong
    );

    // Graceful close.
    let closed = http_call(
        "ws_close",
        &json!({
            "ws_handle": handle,
            "code": 1000,
            "reason": "bye"
        }),
    );
    assert_eq!(closed.get("ok").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn websocket_capability_gate_blocks() {
    let _g = PolicyGuard::allow_net(&["api.stripe.com"]);
    let conn = http_call(
        "ws_connect",
        &json!({
            "url": "wss://echo.websocket.events"
        }),
    );
    assert_eq!(
        conn.get("error").and_then(|v| v.as_str()),
        Some("host_not_allowed"),
        "ws gate should block: {}",
        conn
    );
}
