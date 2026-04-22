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

// ---------------------------------------------------------------------------
// Redirect capability-gate regression tests (issue #128 follow-up).
//
// Before the host-checked redirect policy, a tenant with `allowed.host`
// in its `net.allow` could be 302'd to `disallowed.host` and reqwest
// would follow silently — carrying Authorization / Cookie headers
// with it. These tests pin that behaviour so the gate can't regress:
// the HTTP call must error with `host_not_allowed`, and the
// disallowed host must never receive a byte.
// ---------------------------------------------------------------------------

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};

/// Spawn a tiny HTTP/1.1 server on a random loopback port. The
/// `responder` closure decides what to send back given the raw
/// request-line (e.g. `"GET / HTTP/1.1"`).
///
/// Deliberately hand-rolled so we don't drag hyper into dev-deps
/// just to pin a redirect semantics test.
fn spawn_http_server<F>(hits: Arc<AtomicU32>, responder: F) -> u16
where
    F: Fn(&str) -> String + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind http");
    let port = listener.local_addr().unwrap().port();
    let responder = Arc::new(responder);
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let responder = responder.clone();
            let hits = hits.clone();
            thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                hits.fetch_add(1, Ordering::SeqCst);

                // Read just enough to get the request line + headers.
                // Don't bother parsing the body — we only need to
                // observe that a request landed here.
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                let request_line = raw.lines().next().unwrap_or("").to_string();
                let resp = responder(&request_line);
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
                // Give reqwest a moment to read the response before
                // we drop the socket.
                thread::sleep(Duration::from_millis(20));
            });
        }
    });
    // Small delay to ensure accept() is ready before the test calls
    // `http_call`.
    thread::sleep(Duration::from_millis(50));
    port
}

use std::sync::Arc;

#[test]
fn redirect_to_disallowed_host_is_blocked() {
    // The trap server redirects to the attacker; the attacker server
    // must NEVER see a request.
    let _g = PolicyGuard::allow_net(&["127.0.0.1"]);

    let attacker_hits = Arc::new(AtomicU32::new(0));
    let attacker_port = spawn_http_server(attacker_hits.clone(), |_line| {
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nPWNED!".to_string()
    });

    // The redirector lives on 127.0.0.1 (allowed) and points at
    // `localhost` (NOT allowed — `localhost` != `127.0.0.1` at the
    // capability-gate level; both resolve to loopback but the gate
    // matches on the hostname text).
    let redirector_hits = Arc::new(AtomicU32::new(0));
    let redirector_port = spawn_http_server(redirector_hits.clone(), move |_line| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: http://localhost:{}/secret\r\nContent-Length: 0\r\n\r\n",
            attacker_port
        )
    });

    let url = format!("http://127.0.0.1:{}/trap", redirector_port);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": url,
            "headers": { "Authorization": "Bearer sekret" },
            "timeout_ms": 3000,
        }),
    );

    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(false),
        "should error on disallowed redirect: {}",
        resp
    );
    assert_eq!(
        resp.get("error").and_then(|v| v.as_str()),
        Some("host_not_allowed"),
        "disallowed redirect should map to host_not_allowed: {}",
        resp
    );

    // Absolute guarantee: the attacker never got called.
    assert_eq!(
        attacker_hits.load(Ordering::SeqCst),
        0,
        "attacker received {} request(s) — redirect bypassed the gate!",
        attacker_hits.load(Ordering::SeqCst)
    );
    assert_eq!(
        redirector_hits.load(Ordering::SeqCst),
        1,
        "redirector should have been hit exactly once, got {}",
        redirector_hits.load(Ordering::SeqCst)
    );
}

#[test]
fn redirect_to_allowed_host_is_followed() {
    // Positive control: when the hop lands somewhere still in the
    // allowlist, the follow succeeds and the body comes back.
    let _g = PolicyGuard::allow_net(&["127.0.0.1"]);

    let final_hits = Arc::new(AtomicU32::new(0));
    let final_port = spawn_http_server(final_hits.clone(), |_line| {
        "HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nlanded!".to_string()
    });

    let hop_hits = Arc::new(AtomicU32::new(0));
    let hop_port = spawn_http_server(hop_hits.clone(), move |_line| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/done\r\nContent-Length: 0\r\n\r\n",
            final_port
        )
    });

    let url = format!("http://127.0.0.1:{}/start", hop_port);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": url,
            "timeout_ms": 3000,
        }),
    );

    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "same-host redirect should succeed: {}",
        resp
    );
    assert_eq!(resp.get("status").and_then(|v| v.as_u64()), Some(200));
    assert_eq!(
        resp.get("body").and_then(|v| v.as_str()),
        Some("landed!"),
        "{}",
        resp
    );
    assert_eq!(final_hits.load(Ordering::SeqCst), 1);
    assert_eq!(hop_hits.load(Ordering::SeqCst), 1);
}

#[test]
fn redirect_via_client_handle_also_enforces_gate() {
    // Cover the second client builder path — explicit `client_new`
    // with `max_redirects` used to use `Policy::limited(N)` directly.
    let _g = PolicyGuard::allow_net(&["127.0.0.1"]);

    let attacker_hits = Arc::new(AtomicU32::new(0));
    let attacker_port = spawn_http_server(attacker_hits.clone(), |_line| {
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nPWN".to_string()
    });

    let redirector_port = spawn_http_server(Arc::new(AtomicU32::new(0)), move |_line| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: http://localhost:{}/\r\nContent-Length: 0\r\n\r\n",
            attacker_port
        )
    });

    let client = http_call(
        "client_new",
        &json!({ "max_redirects": 5, "timeout_ms": 3000 }),
    );
    let handle = client
        .get("client_handle")
        .and_then(|v| v.as_u64())
        .expect("client handle");

    let url = format!("http://127.0.0.1:{}/x", redirector_port);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": url,
            "client_handle": handle,
            "headers": { "Cookie": "session=secret" },
        }),
    );

    assert_eq!(
        resp.get("error").and_then(|v| v.as_str()),
        Some("host_not_allowed"),
        "client-handle path must also gate redirects: {}",
        resp
    );
    assert_eq!(
        attacker_hits.load(Ordering::SeqCst),
        0,
        "attacker saw {} hit(s) via explicit client — gate bypassed!",
        attacker_hits.load(Ordering::SeqCst)
    );

    let _ = http_call("client_close", &json!({ "client_handle": handle }));
}

// ---------------------------------------------------------------------------
// ALPN sanity — Amina flagged `http2_prior_knowledge_or_fallback` and we
// want to pin that plain HTTP still works (i.e. the helper does NOT
// force prior-knowledge h2, which would break `http://` URLs by sending
// an h2 preface at a server that speaks HTTP/1.1).
// ---------------------------------------------------------------------------

#[test]
fn plain_http_still_works_not_forced_h2() {
    let _g = PolicyGuard::allow_net(&["127.0.0.1"]);
    let hits = Arc::new(AtomicU32::new(0));
    let port = spawn_http_server(hits.clone(), |line| {
        // Sanity — if reqwest was forcing h2 prior knowledge the
        // server would see an "PRI * HTTP/2.0" preface here, not a
        // normal GET request line.
        assert!(
            line.starts_with("GET "),
            "server saw non-HTTP/1 request line: {:?}",
            line
        );
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_string()
    });

    let url = format!("http://127.0.0.1:{}/ping", port);
    let resp = http_call(
        "request",
        &json!({
            "method": "GET",
            "url": url,
            "timeout_ms": 3000,
        }),
    );

    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "plain http request failed — http2_prior_knowledge_or_fallback may be forcing h2: {}",
        resp
    );
    let version = resp.get("version").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        version.contains("HTTP/1"),
        "expected HTTP/1.x for plain http, got: {} — helper is forcing h2",
        version
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}
