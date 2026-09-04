//! `@deka/http` bridge — outbound HTTP/1.1 + HTTP/2 (ALPN h2), streaming
//! request and response bodies, opt-in per-client cookie jars, and a
//! WebSocket client.
//!
//! Design: modelled after Go's `net/http`. One-liners for the simple
//! case (`http_get` / `http_post`), explicit Request/Response structs
//! for complex work, pluggable Transport (implemented in PHPX, not here)
//! for tests and middleware. Errors are values — no panics, no throws.
//!
//! Every network call is gated by the shared `enforce_net` policy the
//! rest of the runtime already uses (`crates/runtime_core`
//! + `match_rule_item` in `crates/deka_host/src/modules/php/mod.rs`),
//! with one extension: wildcard DNS labels like `*.squareup.com` are
//! accepted in the `net.allow` list. That extension lives next to the
//! existing matcher so TCP / DNS / Redis clients pick it up too.
//!
//! The PHPX side of the module lives in
//! `deka/runtime/php_modules/http/`. Everything here is the Rust
//! plumbing it dispatches through `bridge('http', action, payload)`.
//!
//! Handles are u64 IDs allocated by atomic counters. Each kind
//! (response-stream, client, websocket) has its own map protected by a
//! Mutex. The maps leak a dedicated multi-thread tokio runtime for
//! async I/O, following the same pattern `modules/neo4j.rs` uses so
//! sync ops can call async reqwest / tungstenite without hanging the
//! isolate's own runtime.

use futures_util::{SinkExt, StreamExt};
use reqwest::cookie::Jar;
use reqwest::{Client, ClientBuilder, Method, Response};
use runtime_core::security_policy::SecurityPolicy;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message};
use url::Url;

// ---------------------------------------------------------------------------
// Tokio runtime — shared across all @deka/http work.
// ---------------------------------------------------------------------------

static HTTP_RT: OnceLock<Handle> = OnceLock::new();

fn http_handle() -> &'static Handle {
    HTTP_RT.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("failed to create @deka/http tokio runtime");
        let handle = rt.handle().clone();
        // Leak the runtime — it lives for the process lifetime. Same
        // pattern as neo4j.rs and redis_mod.rs.
        std::mem::forget(rt);
        handle
    })
}

/// Run an async future to completion, blocking the calling thread. The
/// future runs on the shared HTTP runtime's workers so I/O progresses
/// independently.
pub(crate) fn block_on<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handle = http_handle();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    handle.spawn(async move {
        let result = f.await;
        let _ = tx.send(result);
    });
    rx.recv().expect("@deka/http async task failed")
}

// ---------------------------------------------------------------------------
// Handles + registries.
// ---------------------------------------------------------------------------

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn new_handle() -> u64 {
    NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
struct ClientEntry {
    client: Client,
    /// Kept alongside the client so PHPX callers can expose a
    /// `$client->jar()->cookies()` view for debugging. `None` when the
    /// caller opted out of cookies (the default).
    jar: Option<Arc<Jar>>,
    /// Max redirects the builder was configured with — stashed so we
    /// can surface it in client debug info. The client itself already
    /// honours this through reqwest's redirect::Policy.
    #[allow(dead_code)]
    max_redirects: usize,
    /// Default timeout stashed for introspection. The timeout is also
    /// already baked into the `Client` above, so per-request calls
    /// don't need to re-apply it.
    #[allow(dead_code)]
    timeout_ms: Option<u64>,
}

static CLIENTS: OnceLock<Mutex<HashMap<u64, ClientEntry>>> = OnceLock::new();

fn clients() -> &'static Mutex<HashMap<u64, ClientEntry>> {
    CLIENTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Response-body stream state. Created when a PHPX caller asks for a
/// streaming response; dropped when they call `stream_close` or the
/// full body is drained through `stream_read`.
struct ResponseStream {
    status: u16,
    headers: Vec<(String, String)>,
    version: String,
    final_url: String,
    body_rx: mpsc::Receiver<Result<Vec<u8>, String>>,
    done: bool,
}

static RESP_STREAMS: OnceLock<Mutex<HashMap<u64, ResponseStream>>> = OnceLock::new();

fn resp_streams() -> &'static Mutex<HashMap<u64, ResponseStream>> {
    RESP_STREAMS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Request-body stream state. The PHPX caller pushes chunks with
/// `req_stream_write`, closes with `req_stream_end`, then dispatches
/// the request which pulls from the receiver side via reqwest's
/// `Body::wrap_stream`. Each producer has its own channel.
struct RequestStreamProducer {
    tx: mpsc::Sender<Result<bytes::Bytes, String>>,
}

static REQ_STREAMS: OnceLock<Mutex<HashMap<u64, RequestStreamProducer>>> = OnceLock::new();

fn req_streams() -> &'static Mutex<HashMap<u64, RequestStreamProducer>> {
    REQ_STREAMS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A pending request-body consumer: the other half of the
/// `RequestStreamProducer` channel. Held until dispatch attaches it to
/// a reqwest body.
struct RequestStreamConsumer {
    rx: mpsc::Receiver<Result<bytes::Bytes, String>>,
}

static REQ_STREAM_CONSUMERS: OnceLock<Mutex<HashMap<u64, RequestStreamConsumer>>> = OnceLock::new();

fn req_stream_consumers() -> &'static Mutex<HashMap<u64, RequestStreamConsumer>> {
    REQ_STREAM_CONSUMERS.get_or_init(|| Mutex::new(HashMap::new()))
}

// WebSocket handle — we split the connection into a send half and a
// recv half and store them behind Mutexes so PHPX can send + recv
// concurrently without the bridge's single-threaded call pattern
// deadlocking on itself.
type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;
type WsStream = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

struct WsEntry {
    sink: Arc<tokio::sync::Mutex<WsSink>>,
    stream: Arc<tokio::sync::Mutex<WsStream>>,
    /// Configurable per-frame size limit. Frames larger than this
    /// produce a `WsError::frame_too_large`.
    max_frame_bytes: usize,
}

static WEBSOCKETS: OnceLock<Mutex<HashMap<u64, WsEntry>>> = OnceLock::new();

fn websockets() -> &'static Mutex<HashMap<u64, WsEntry>> {
    WEBSOCKETS.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Capability gate — host extracted from URL, matched against the
// existing `net` allowlist. The matcher lives in `php/mod.rs` so other
// net ops share it; we only extract+normalize the host here.
// ---------------------------------------------------------------------------

/// Returns the host portion of the URL in lowercase, with no port.
/// `None` if the URL can't be parsed.
fn host_of(url_str: &str) -> Option<String> {
    Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// Check a URL's host against the tenant's `net` allowlist. Returns
/// `Ok(())` when allowed, `Err(reason)` otherwise. Privileged code
/// (platform server / framework) is already exempted by the shared
/// `enforce_net` helper. The policy is passed in so callers can pin it
/// instead of racing on the process-global `DEKA_SECURITY_POLICY` env var
/// (deka#537).
pub(crate) fn enforce_host_allowed_with(
    policy: &SecurityPolicy,
    url_str: &str,
) -> Result<(), String> {
    let host = host_of(url_str).ok_or_else(|| format!("invalid url: '{}'", url_str))?;
    match crate::modules::php::enforce_net_public_with(policy, &host) {
        Ok(()) => Ok(()),
        Err(err) => Err(format!("host_not_allowed: {} ({})", host, err)),
    }
}

/// Build a redirect policy that re-runs the capability gate on every
/// hop. Fixes a capability-gate bypass where reqwest's default
/// `Policy::limited(N)` would follow 3xx across origins without
/// re-checking the host — a 302 from an allowlisted host to an
/// attacker host used to sail through, carrying Authorization /
/// Cookie headers with it.
///
/// `max_hops` caps the follow chain (matching the previous
/// `Policy::limited(N)` behaviour). When the next hop's host fails
/// the gate we `stop()` — reqwest surfaces this as a `redirect`
/// error which `encode_reqwest_error` maps to `too_many_redirects`;
/// we upgrade that to an explicit `host_not_allowed` classification
/// in the dispatch path by inspecting the error chain.
fn host_checked_redirect_policy(
    policy: SecurityPolicy,
    max_hops: usize,
) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        // Snapshot what we need from `attempt` before any terminal
        // call consumes it — `attempt.error()` / `attempt.follow()`
        // / `attempt.stop()` all take `self` by value.
        let host = attempt.url().host_str().unwrap_or("").to_ascii_lowercase();
        let url = attempt.url().to_string();

        if attempt.previous().len() >= max_hops {
            return attempt.error(RedirectHostDenied {
                host,
                reason: format!("too many redirects (> {})", max_hops),
            });
        }
        if let Err(err) = enforce_host_allowed_with(&policy, &url) {
            return attempt.error(RedirectHostDenied { host, reason: err });
        }
        attempt.follow()
    })
}

/// Marker error type surfaced by `host_checked_redirect_policy` when a
/// redirect hop is refused. reqwest wraps this inside its own
/// `reqwest::Error` (kind = redirect); we walk the source chain in
/// `encode_reqwest_error` to detect it and promote the response
/// classification from the generic `too_many_redirects` to
/// `host_not_allowed`, which matches what the initial-URL gate
/// already returns for a disallowed host.
#[derive(Debug)]
struct RedirectHostDenied {
    host: String,
    reason: String,
}

impl std::fmt::Display for RedirectHostDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "redirect host not allowed: {} ({})",
            self.host, self.reason
        )
    }
}

impl std::error::Error for RedirectHostDenied {}

// ---------------------------------------------------------------------------
// Entry point dispatched from the pool bridge.
// ---------------------------------------------------------------------------

/// Public entry — called from `op_deka_http_call` which the pool's
/// bridge layer routes `bridge('http', action, payload)` through.
/// Reads `DEKA_SECURITY_POLICY` once per call; use
/// `http_call_with_policy` to pin the policy instead.
pub fn http_call(action: &str, payload: &Value) -> Value {
    http_call_with_policy(
        &crate::modules::php::security_policy_from_env(),
        action,
        payload,
    )
}

/// The dispatch itself, with the policy passed in. See `http_call` for
/// why this exists.
pub fn http_call_with_policy(policy: &SecurityPolicy, action: &str, payload: &Value) -> Value {
    match action {
        "request" => request_with_policy(policy, payload),
        "stream_read" => stream_read(payload),
        "stream_close" => stream_close(payload),
        "req_stream_new" => req_stream_new(payload),
        "req_stream_write" => req_stream_write(payload),
        "req_stream_end" => req_stream_end(payload),
        "client_new" => client_new_with_policy(policy, payload),
        "client_close" => client_close(payload),
        "client_cookies" => client_cookies(payload),
        "ws_connect" => ws_connect_with_policy(policy, payload),
        "ws_send_text" => ws_send_text(payload),
        "ws_send_binary" => ws_send_binary(payload),
        "ws_recv" => ws_recv(payload),
        "ws_ping" => ws_ping(payload),
        "ws_close" => ws_close(payload),
        other => json!({ "ok": false, "error": format!("unknown http action '{}'", other) }),
    }
}

// ---------------------------------------------------------------------------
// HTTP request — buffered or streaming.
// ---------------------------------------------------------------------------

fn request_with_policy(policy: &SecurityPolicy, payload: &Value) -> Value {
    let url_str = payload
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url_str.is_empty() {
        return json!({ "ok": false, "error": "invalid_url", "message": "url is required" });
    }
    if let Err(err) = enforce_host_allowed_with(policy, &url_str) {
        return json!({ "ok": false, "error": "host_not_allowed", "message": err });
    }

    let method_str = payload
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let method = match Method::from_bytes(method_str.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return json!({ "ok": false, "error": "invalid_method", "message": method_str });
        }
    };

    let headers_map = payload
        .get("headers")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let timeout_ms = payload.get("timeout_ms").and_then(|v| v.as_u64());
    let stream_response = payload
        .get("stream_response")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let body_max = payload
        .get("max_body_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(16 * 1024 * 1024); // 16 MB default cap for buffered responses
    let response_as_bytes = payload
        .get("response_as_bytes")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let client_handle = payload.get("client_handle").and_then(|v| v.as_u64());
    let client_entry = match client_handle {
        Some(h) => match clients().lock().ok().and_then(|g| g.get(&h).cloned()) {
            Some(c) => Some(c),
            None => {
                return json!({ "ok": false, "error": "invalid_client_handle", "message": h });
            }
        },
        None => None,
    };

    // Resolve request body. Three shapes:
    // 1. body_stream_handle → pulls bytes from the request-stream queue
    // 2. body_bytes (array or Uint8Array-like) → buffered bytes body
    // 3. body (string) → buffered string body
    enum BodyKind {
        Buffered(Vec<u8>),
        Stream(u64),
        None,
    }
    let body_kind = if let Some(handle) = payload.get("body_stream_handle").and_then(|v| v.as_u64())
    {
        BodyKind::Stream(handle)
    } else if let Some(arr) = payload.get("body_bytes").and_then(|v| v.as_array()) {
        let bytes: Vec<u8> = arr
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n.min(255) as u8))
            .collect();
        BodyKind::Buffered(bytes)
    } else if let Some(s) = payload.get("body").and_then(|v| v.as_str()) {
        BodyKind::Buffered(s.as_bytes().to_vec())
    } else {
        BodyKind::None
    };

    // If streaming body, detach the consumer side of the channel now.
    let stream_consumer = if let BodyKind::Stream(handle) = &body_kind {
        match req_stream_consumers()
            .lock()
            .ok()
            .and_then(|mut g| g.remove(handle))
        {
            Some(c) => Some(c),
            None => {
                return json!({ "ok": false, "error": "invalid_stream_handle", "message": handle });
            }
        }
    } else {
        None
    };

    // Owned copy for the `'static` async block (redirect re-checks).
    let policy = policy.clone();
    block_on(async move {
        // Build or reuse the client.
        let client = if let Some(entry) = &client_entry {
            entry.client.clone()
        } else {
            let mut b = ClientBuilder::new()
                .use_rustls_tls()
                .tcp_nodelay(true)
                // HTTP/2 negotiated via ALPN when server supports it;
                // HTTP/1.1 otherwise. Server push disabled — we're a client.
                .http2_prior_knowledge_or_fallback()
                // Re-run the capability gate per redirect hop. The
                // default `Policy::limited(N)` would silently follow a
                // 302 to an attacker host and ship Authorization /
                // Cookie headers with it — see issue #128 review.
                .redirect(host_checked_redirect_policy(policy.clone(), 10));
            if let Some(t) = timeout_ms {
                b = b.timeout(Duration::from_millis(t));
            }
            match b.build() {
                Ok(c) => c,
                Err(e) => {
                    return json!({ "ok": false, "error": "transport_error", "message": format!("client build: {}", e) });
                }
            }
        };

        let mut req = client.request(method, &url_str);
        for (k, v) in headers_map.iter() {
            if let Some(vs) = v.as_str() {
                req = req.header(k.as_str(), vs);
            }
        }
        if let Some(t) = timeout_ms {
            req = req.timeout(Duration::from_millis(t));
        }

        req = match body_kind {
            BodyKind::Buffered(b) => req.body(b),
            BodyKind::None => req,
            BodyKind::Stream(_) => {
                let consumer = stream_consumer.expect("stream consumer detached above");
                let stream = tokio_stream::wrappers::ReceiverStream::new(consumer.rx);
                let body = reqwest::Body::wrap_stream(stream);
                req.body(body)
            }
        };

        let resp: Response = match req.send().await {
            Ok(r) => r,
            Err(e) => return encode_reqwest_error(&e),
        };

        let status = resp.status().as_u16();
        let version = format!("{:?}", resp.version());
        let final_url = resp.url().to_string();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        if stream_response {
            // Spawn a pump that pushes chunks into a channel. The PHPX
            // caller drains it through `stream_read`.
            let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>(16);
            tokio::spawn(async move {
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(bytes) => {
                            if tx.send(Ok(bytes.to_vec())).await.is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(format!("stream_chunk_error: {}", e))).await;
                            return;
                        }
                    }
                }
            });
            let stream_handle = new_handle();
            resp_streams().lock().unwrap().insert(
                stream_handle,
                ResponseStream {
                    status,
                    headers: headers.clone(),
                    version: version.clone(),
                    final_url: final_url.clone(),
                    body_rx: rx,
                    done: false,
                },
            );
            return json!({
                "ok": true,
                "streamed": true,
                "stream_handle": stream_handle,
                "status": status,
                "version": version,
                "final_url": final_url,
                "headers": headers_to_json(&headers),
            });
        }

        // Buffered path — fetch with a size cap.
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => return encode_reqwest_error(&e),
        };
        if (bytes.len() as u64) > body_max {
            return json!({
                "ok": false,
                "error": "body_too_large",
                "message": format!("response body {} > max {}", bytes.len(), body_max),
            });
        }

        let mut out = json!({
            "ok": true,
            "streamed": false,
            "status": status,
            "version": version,
            "final_url": final_url,
            "headers": headers_to_json(&headers),
        });
        if response_as_bytes {
            out["body_bytes"] = Value::Array(bytes.iter().map(|b| Value::from(*b)).collect());
        } else {
            // Best-effort UTF-8 decode. Non-UTF-8 callers pass
            // response_as_bytes=true explicitly.
            let body = String::from_utf8_lossy(&bytes).to_string();
            out["body"] = Value::String(body);
        }
        out
    })
}

fn encode_reqwest_error(e: &reqwest::Error) -> Value {
    // Walk the error source chain looking for our redirect-host-gate
    // refusal marker. reqwest wraps the callback error inside its own
    // `reqwest::Error` (kind = redirect); we promote that to the same
    // `host_not_allowed` classification the initial-URL gate returns
    // so PHPX callers don't have to distinguish "disallowed initial
    // host" from "disallowed redirect hop".
    {
        let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
        while let Some(cause) = src {
            if let Some(denied) =
                <dyn std::error::Error + 'static>::downcast_ref::<RedirectHostDenied>(cause)
            {
                return json!({
                    "ok": false,
                    "error": "host_not_allowed",
                    "message": denied.to_string(),
                    "host": denied.host.clone(),
                });
            }
            src = std::error::Error::source(cause);
        }
    }

    let msg = e.to_string();
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect_refused"
    } else if e.is_redirect() {
        "too_many_redirects"
    } else if e.is_decode() {
        "decode_error"
    } else if e.is_builder() {
        "invalid_url"
    } else if msg.contains("tls") || msg.contains("TLS") {
        "tls_error"
    } else if msg.contains("dns") || msg.contains("resolve") {
        "dns_error"
    } else {
        "transport_error"
    };
    json!({ "ok": false, "error": kind, "message": msg })
}

fn headers_to_json(headers: &[(String, String)]) -> Value {
    let mut obj = serde_json::Map::new();
    for (k, v) in headers {
        obj.insert(k.clone(), Value::String(v.clone()));
    }
    Value::Object(obj)
}

// ---------------------------------------------------------------------------
// Response streaming — stream_read / stream_close.
// ---------------------------------------------------------------------------

fn stream_read(payload: &Value) -> Value {
    let handle = match payload.get("stream_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_stream_handle" }),
    };
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(30_000);

    // Pull the Receiver out of the map, await on it, then put it back.
    // std Mutex is not Send across awaits, so we never hold it across
    // .await — instead we remove+reinsert the ResponseStream entry.
    let removed = {
        let mut guard = match resp_streams().lock() {
            Ok(g) => g,
            Err(_) => return json!({ "ok": false, "error": "stream_lock_poisoned" }),
        };
        match guard.remove(&handle) {
            Some(e) => e,
            None => return json!({ "ok": false, "error": "invalid_stream_handle" }),
        }
    };

    if removed.done {
        // Put it back so close() can still clean up and repeated reads
        // keep seeing done=true.
        let mut entry = removed;
        entry.done = true;
        resp_streams().lock().unwrap().insert(handle, entry);
        return json!({ "ok": true, "done": true });
    }

    let status = removed.status;
    let version = removed.version.clone();
    let final_url = removed.final_url.clone();
    let headers = removed.headers.clone();

    let (result, entry_back) = block_on(async move {
        let mut entry = removed;
        let fut = entry.body_rx.recv();
        let got = tokio::time::timeout(Duration::from_millis(timeout_ms.max(1)), fut).await;
        let value = match got {
            Ok(Some(Ok(chunk))) => {
                let arr: Vec<Value> = chunk.iter().map(|b| Value::from(*b)).collect();
                json!({
                    "ok": true,
                    "done": false,
                    "chunk": arr,
                    "status": status,
                    "version": version,
                    "final_url": final_url,
                    "headers": headers_to_json(&headers),
                })
            }
            Ok(Some(Err(err))) => {
                entry.done = true;
                json!({ "ok": false, "error": "stream_error", "message": err })
            }
            Ok(None) => {
                entry.done = true;
                json!({ "ok": true, "done": true })
            }
            Err(_) => {
                json!({ "ok": false, "error": "timeout" })
            }
        };
        (value, entry)
    });

    // Reinsert the entry so subsequent reads continue draining.
    resp_streams().lock().unwrap().insert(handle, entry_back);
    result
}

fn stream_close(payload: &Value) -> Value {
    let handle = match payload.get("stream_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_stream_handle" }),
    };
    resp_streams().lock().unwrap().remove(&handle);
    json!({ "ok": true })
}

// ---------------------------------------------------------------------------
// Request body streaming.
// ---------------------------------------------------------------------------

fn req_stream_new(_payload: &Value) -> Value {
    let (tx, rx) = mpsc::channel::<Result<bytes::Bytes, String>>(16);
    let handle = new_handle();
    req_streams()
        .lock()
        .unwrap()
        .insert(handle, RequestStreamProducer { tx });
    req_stream_consumers()
        .lock()
        .unwrap()
        .insert(handle, RequestStreamConsumer { rx });
    json!({ "ok": true, "stream_handle": handle })
}

fn req_stream_write(payload: &Value) -> Value {
    let handle = match payload.get("stream_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_stream_handle" }),
    };
    let chunk: Vec<u8> = if let Some(arr) = payload.get("chunk").and_then(|v| v.as_array()) {
        arr.iter()
            .filter_map(|v| v.as_u64().map(|n| n.min(255) as u8))
            .collect()
    } else if let Some(s) = payload.get("chunk").and_then(|v| v.as_str()) {
        s.as_bytes().to_vec()
    } else {
        return json!({ "ok": false, "error": "invalid_chunk" });
    };

    let tx_opt = req_streams()
        .lock()
        .unwrap()
        .get(&handle)
        .map(|p| p.tx.clone());
    let Some(tx) = tx_opt else {
        return json!({ "ok": false, "error": "invalid_stream_handle" });
    };

    block_on(async move {
        match tx.send(Ok(bytes::Bytes::from(chunk))).await {
            Ok(()) => json!({ "ok": true }),
            Err(_) => json!({ "ok": false, "error": "stream_closed" }),
        }
    })
}

fn req_stream_end(payload: &Value) -> Value {
    let handle = match payload.get("stream_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_stream_handle" }),
    };
    req_streams().lock().unwrap().remove(&handle);
    json!({ "ok": true })
}

// ---------------------------------------------------------------------------
// HttpClient — persistent config + optional cookie jar.
// ---------------------------------------------------------------------------

fn client_new_with_policy(policy: &SecurityPolicy, payload: &Value) -> Value {
    let timeout_ms = payload.get("timeout_ms").and_then(|v| v.as_u64());
    let max_redirects = payload
        .get("max_redirects")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;
    let enable_jar = payload
        .get("cookie_jar")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut b = ClientBuilder::new()
        .use_rustls_tls()
        .tcp_nodelay(true)
        .http2_prior_knowledge_or_fallback()
        // `Policy::limited` follows redirects without re-checking the
        // host — that lets an allowlisted origin 302 us at an
        // attacker. Use a custom policy that re-enforces the
        // capability gate on every hop. See issue #128 review.
        .redirect(host_checked_redirect_policy(policy.clone(), max_redirects));
    if let Some(t) = timeout_ms {
        b = b.timeout(Duration::from_millis(t));
    }
    let jar = if enable_jar {
        let jar = Arc::new(Jar::default());
        b = b.cookie_provider(jar.clone());
        Some(jar)
    } else {
        None
    };

    let client = match b.build() {
        Ok(c) => c,
        Err(e) => {
            return json!({ "ok": false, "error": "transport_error", "message": e.to_string() });
        }
    };

    let handle = new_handle();
    clients().lock().unwrap().insert(
        handle,
        ClientEntry {
            client,
            jar,
            max_redirects,
            timeout_ms,
        },
    );
    json!({ "ok": true, "client_handle": handle })
}

fn client_close(payload: &Value) -> Value {
    let handle = match payload.get("client_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_client_handle" }),
    };
    clients().lock().unwrap().remove(&handle);
    json!({ "ok": true })
}

/// Inspect the jar for a given URL. Returns an array of cookie entries
/// the jar would send with a request to that URL — scoped by
/// Domain/Path/Secure just like the browser model.
fn client_cookies(payload: &Value) -> Value {
    let handle = match payload.get("client_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_client_handle" }),
    };
    let url_str = payload
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url_str.is_empty() {
        return json!({ "ok": false, "error": "invalid_url" });
    }
    let entry = match clients().lock().ok().and_then(|g| g.get(&handle).cloned()) {
        Some(e) => e,
        None => return json!({ "ok": false, "error": "invalid_client_handle" }),
    };
    let Some(jar) = entry.jar else {
        return json!({ "ok": true, "cookies": Value::Array(vec![]) });
    };
    let url = match Url::parse(&url_str) {
        Ok(u) => u,
        Err(_) => return json!({ "ok": false, "error": "invalid_url" }),
    };
    use reqwest::cookie::CookieStore;
    let cookies: Vec<Value> = match jar.cookies(&url) {
        Some(header) => {
            let raw = header.to_str().unwrap_or("");
            raw.split(';')
                .filter_map(|pair| {
                    let trimmed = pair.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    let (name, value) = trimmed.split_once('=')?;
                    Some(json!({ "name": name.trim(), "value": value.trim() }))
                })
                .collect()
        }
        None => vec![],
    };
    json!({ "ok": true, "cookies": Value::Array(cookies) })
}

// ---------------------------------------------------------------------------
// WebSocket client.
// ---------------------------------------------------------------------------

fn ws_connect_with_policy(policy: &SecurityPolicy, payload: &Value) -> Value {
    let url_str = payload
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url_str.is_empty() {
        return json!({ "ok": false, "error": "invalid_url" });
    }
    if let Err(err) = enforce_host_allowed_with(policy, &url_str) {
        return json!({ "ok": false, "error": "host_not_allowed", "message": err });
    }

    let max_frame_bytes = payload
        .get("max_frame_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(10 * 1024 * 1024) as usize;

    let headers_map = payload
        .get("headers")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    block_on(async move {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = match url_str.as_str().into_client_request() {
            Ok(r) => r,
            Err(e) => {
                return json!({ "ok": false, "error": "invalid_url", "message": e.to_string() });
            }
        };
        for (k, v) in headers_map.iter() {
            if let Some(vs) = v.as_str() {
                if let (Ok(name), Ok(value)) = (
                    tokio_tungstenite::tungstenite::http::HeaderName::try_from(k.as_str()),
                    tokio_tungstenite::tungstenite::http::HeaderValue::try_from(vs),
                ) {
                    request.headers_mut().insert(name, value);
                }
            }
        }

        let (ws_stream, _resp) = match tokio_tungstenite::connect_async(request).await {
            Ok(x) => x,
            Err(e) => {
                return json!({ "ok": false, "error": "ws_connect_failed", "message": e.to_string() });
            }
        };
        let (sink, stream) = ws_stream.split();
        let handle = new_handle();
        websockets().lock().unwrap().insert(
            handle,
            WsEntry {
                sink: Arc::new(tokio::sync::Mutex::new(sink)),
                stream: Arc::new(tokio::sync::Mutex::new(stream)),
                max_frame_bytes,
            },
        );
        json!({ "ok": true, "ws_handle": handle })
    })
}

fn ws_send_text(payload: &Value) -> Value {
    let handle = match payload.get("ws_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    let text = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sink = match websockets()
        .lock()
        .ok()
        .and_then(|g| g.get(&handle).map(|e| e.sink.clone()))
    {
        Some(s) => s,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    block_on(async move {
        let mut sink = sink.lock().await;
        match sink.send(Message::Text(text)).await {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": "ws_send_failed", "message": e.to_string() }),
        }
    })
}

fn ws_send_binary(payload: &Value) -> Value {
    let handle = match payload.get("ws_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    let bytes: Vec<u8> = if let Some(arr) = payload.get("bytes").and_then(|v| v.as_array()) {
        arr.iter()
            .filter_map(|v| v.as_u64().map(|n| n.min(255) as u8))
            .collect()
    } else if let Some(s) = payload.get("bytes").and_then(|v| v.as_str()) {
        s.as_bytes().to_vec()
    } else {
        return json!({ "ok": false, "error": "invalid_bytes" });
    };
    let sink = match websockets()
        .lock()
        .ok()
        .and_then(|g| g.get(&handle).map(|e| e.sink.clone()))
    {
        Some(s) => s,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    block_on(async move {
        let mut sink = sink.lock().await;
        match sink.send(Message::Binary(bytes)).await {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": "ws_send_failed", "message": e.to_string() }),
        }
    })
}

fn ws_recv(payload: &Value) -> Value {
    let handle = match payload.get("ws_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(30_000);
    let (stream, sink, max_frame) = match websockets().lock().ok().and_then(|g| {
        g.get(&handle)
            .map(|e| (e.stream.clone(), e.sink.clone(), e.max_frame_bytes))
    }) {
        Some(x) => x,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    block_on(async move {
        let mut stream = stream.lock().await;
        let fut = stream.next();
        let msg = match tokio::time::timeout(Duration::from_millis(timeout_ms), fut).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => {
                return json!({ "ok": false, "error": "ws_recv_failed", "message": e.to_string() });
            }
            Ok(None) => return json!({ "ok": true, "kind": "close", "code": 1006, "reason": "" }),
            Err(_) => return json!({ "ok": false, "error": "timeout" }),
        };
        match msg {
            Message::Text(t) => {
                if t.len() > max_frame {
                    return json!({ "ok": false, "error": "frame_too_large" });
                }
                json!({ "ok": true, "kind": "text", "text": t })
            }
            Message::Binary(b) => {
                if b.len() > max_frame {
                    return json!({ "ok": false, "error": "frame_too_large" });
                }
                let arr: Vec<Value> = b.iter().map(|x| Value::from(*x)).collect();
                json!({ "ok": true, "kind": "binary", "bytes": arr })
            }
            Message::Ping(p) => {
                // Auto-pong for keepalive.
                let mut sink = sink.lock().await;
                let _ = sink.send(Message::Pong(p.clone())).await;
                let arr: Vec<Value> = p.iter().map(|x| Value::from(*x)).collect();
                json!({ "ok": true, "kind": "ping", "bytes": arr })
            }
            Message::Pong(p) => {
                let arr: Vec<Value> = p.iter().map(|x| Value::from(*x)).collect();
                json!({ "ok": true, "kind": "pong", "bytes": arr })
            }
            Message::Close(frame) => {
                let (code, reason) = match frame {
                    Some(f) => (u16::from(f.code), f.reason.to_string()),
                    None => (1000, String::new()),
                };
                json!({ "ok": true, "kind": "close", "code": code, "reason": reason })
            }
            Message::Frame(_) => json!({ "ok": true, "kind": "frame", "text": "" }),
        }
    })
}

fn ws_ping(payload: &Value) -> Value {
    let handle = match payload.get("ws_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    let payload_bytes: Vec<u8> = if let Some(arr) = payload.get("bytes").and_then(|v| v.as_array())
    {
        arr.iter()
            .filter_map(|v| v.as_u64().map(|n| n.min(255) as u8))
            .collect()
    } else {
        Vec::new()
    };
    let sink = match websockets()
        .lock()
        .ok()
        .and_then(|g| g.get(&handle).map(|e| e.sink.clone()))
    {
        Some(s) => s,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    block_on(async move {
        let mut sink = sink.lock().await;
        match sink.send(Message::Ping(payload_bytes)).await {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": "ws_send_failed", "message": e.to_string() }),
        }
    })
}

fn ws_close(payload: &Value) -> Value {
    let handle = match payload.get("ws_handle").and_then(|v| v.as_u64()) {
        Some(h) => h,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    let code = payload.get("code").and_then(|v| v.as_u64()).unwrap_or(1000) as u16;
    let reason = payload
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let entry = match websockets()
        .lock()
        .ok()
        .and_then(|g| g.get(&handle).map(|e| (e.sink.clone(), e.stream.clone())))
    {
        Some(x) => x,
        None => return json!({ "ok": false, "error": "invalid_ws_handle" }),
    };
    let (sink, _stream) = entry;
    let result = block_on(async move {
        let mut sink = sink.lock().await;
        let close = Message::Close(Some(CloseFrame {
            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(code),
            reason: reason.into(),
        }));
        let _ = sink.send(close).await;
        let _ = sink.close().await;
    });
    let _ = result;
    websockets().lock().unwrap().remove(&handle);
    json!({ "ok": true })
}

// ---------------------------------------------------------------------------
// Small trait shim — reqwest's ClientBuilder doesn't ship a single
// "try h2, fallback to h1" helper, so we provide one here. The effect:
// ALPN advertises h2+http/1.1, servers pick h2 when they support it,
// clients silently fall back to HTTP/1.1 otherwise. This is the
// default reqwest behaviour; we make it explicit so future edits
// don't accidentally force-upgrade.
// ---------------------------------------------------------------------------

trait ClientBuilderExt {
    fn http2_prior_knowledge_or_fallback(self) -> Self;
}

impl ClientBuilderExt for ClientBuilder {
    fn http2_prior_knowledge_or_fallback(self) -> Self {
        // reqwest 0.11 default: ALPN-negotiated h2 for HTTPS, HTTP/1.1
        // for plaintext. We just enable the adaptive window for h2 so
        // large downloads don't starve on flow-control.
        self.http2_adaptive_window(true)
    }
}
