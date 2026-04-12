//! Lightweight page-view tracking for storefront HTML responses.
//!
//! Every multi-tenant HTTP response that is (a) a successful 2xx and (b) has a
//! `Content-Type` of `text/html` is counted into Redis, scoped per shop. The
//! data is consumed by the merchant admin (`tana-store-admin`) through the
//! analytics endpoints.
//!
//! ## Design
//!
//! The tracker is fire-and-forget and MUST NOT block the request path:
//!
//! 1. On first use we spawn a dedicated background OS thread (via
//!    [`std::sync::OnceLock`] + [`std::thread::spawn`]) that owns the Redis
//!    connection. A plain OS thread — not a tokio task — so the blocking
//!    `redis::Connection` calls cannot stall the async runtime.
//! 2. Request handlers call [`track_pageview`] which filters the response
//!    and, if eligible, pushes a [`PageviewEvent`] onto a bounded channel
//!    using `try_send`. If the channel is full or the worker is gone the
//!    event is dropped — we never await, never error, never fail the request.
//! 3. The background thread drains events and issues `INCR` / `EXPIRE` on
//!    Redis. If Redis is down it reconnects lazily and keeps draining.
//!
//! ## Redis key shape
//!
//! - `analytics:{shop_id}:pageviews:total` — lifetime counter.
//! - `analytics:{shop_id}:pageviews:{YYYY-MM-DD}` — daily counter, TTL 90 days
//!   (set/refreshed on every increment).
//!
//! ## Filter
//!
//! We identify "page views" by the response `Content-Type`, not by URL, so
//! every PHPX-generated HTML page is counted regardless of route. Assets,
//! JSON APIs, redirects, and error responses are skipped entirely.

use std::collections::HashMap;
use std::env;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Bounded channel capacity. Under normal traffic events are drained on the
/// order of milliseconds — this buffer exists only to absorb short bursts.
/// If we overflow we drop the excess silently.
const CHANNEL_CAPACITY: usize = 4096;

/// TTL applied to the daily per-shop counter key, matching the 90-day window
/// the admin UI shows historically.
const DAILY_TTL_SECS: u64 = 90 * 24 * 60 * 60;

/// A single page-view event produced by the request path.
///
/// We carry the full request header set rather than a resolved `shop_id` so
/// that the `subdomain → shop_id` Redis lookup happens on the worker thread,
/// not the hot async request path. The worker already owns a Redis
/// connection so resolution is essentially free alongside the INCR pipeline.
#[derive(Debug, Clone)]
pub struct PageviewEvent {
    /// Exactly the `(name, value)` header pairs that were presented to the
    /// runtime — only the `Host` / `X-Shop-ID` entries are read, but we keep
    /// the vec shape so the existing `pool::tenant::resolve_tenant_from_headers`
    /// can be called directly.
    pub headers: Vec<(String, String)>,
    /// `YYYY-MM-DD` in UTC, computed at emit time so late draining cannot
    /// mis-bucket a burst that crosses midnight.
    pub day: String,
}

static SENDER: OnceLock<mpsc::SyncSender<PageviewEvent>> = OnceLock::new();

/// Initialise the background pageview worker (lazy, once-per-process).
fn ensure_worker() -> &'static mpsc::SyncSender<PageviewEvent> {
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<PageviewEvent>(CHANNEL_CAPACITY);
        thread::Builder::new()
            .name("deka-pageviews".to_string())
            .spawn(move || run_worker(rx))
            .expect("spawn pageview worker thread");
        tx
    })
}

/// Returns today's date in `YYYY-MM-DD` (UTC).
fn today_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Returns true if `headers` describe a successful HTML response that should
/// be counted:
/// - status is 2xx
/// - `Content-Type` media type is exactly `text/html` (case-insensitive,
///   parameters after `;` ignored)
///
/// The key comparison is tolerant of a pre-existing bug in the JS→Rust
/// response-envelope bridge that can emit header names wrapped in literal
/// single or double quotes (e.g. `'content-type'`). We strip a matching
/// pair of surrounding quotes before comparing.
pub fn should_track(status: u16, headers: &HashMap<String, String>) -> bool {
    if !(200..300).contains(&status) {
        return false;
    }
    for (key, value) in headers {
        if header_name_is_content_type(key) {
            let lowered = value.to_ascii_lowercase();
            let media = lowered.split(';').next().unwrap_or("").trim();
            return media == "text/html";
        }
    }
    false
}

fn header_name_is_content_type(key: &str) -> bool {
    let trimmed = key.trim();
    // Strip one matching pair of surrounding quotes if present.
    let unquoted = match (trimmed.chars().next(), trimmed.chars().last()) {
        (Some('\''), Some('\'')) | (Some('"'), Some('"')) if trimmed.len() >= 2 => {
            &trimmed[1..trimmed.len() - 1]
        }
        _ => trimmed,
    };
    unquoted.eq_ignore_ascii_case("content-type")
}

/// Try to record a page-view.
///
/// Cheap and non-blocking: filters on `status` + `response_headers`, and if
/// eligible captures the request headers (needed for tenant resolution) and
/// pushes one event onto an in-memory channel. No Redis I/O happens on the
/// request path — the worker thread handles subdomain → shop_id resolution
/// and the counter writes.
///
/// Returns `true` if the event was queued, `false` if it was filtered or
/// dropped (full channel, closed worker, etc).
pub fn track_pageview(
    request_headers: &[(String, String)],
    status: u16,
    response_headers: &HashMap<String, String>,
) -> bool {
    if !should_track(status, response_headers) {
        return false;
    }
    let event = PageviewEvent {
        headers: request_headers.to_vec(),
        day: today_utc(),
    };
    let sender = ensure_worker();
    match sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(_)) => {
            tracing::trace!("pageview channel full, dropping event");
            false
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            tracing::trace!("pageview channel closed, dropping event");
            false
        }
    }
}

/// The background thread body: drains the channel and writes to Redis.
fn run_worker(rx: mpsc::Receiver<PageviewEvent>) {
    let redis_url =
        env::var("DEKA_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());
    tracing::info!("pageview tracker online, redis={}", redis_url);

    let mut conn: Option<redis::Connection> = None;

    while let Ok(event) = rx.recv() {
        // Resolve subdomain → shop_id. The pool's tenant resolver keeps a
        // thread-local Redis connection cache — on this dedicated worker
        // thread the first call establishes it and subsequent calls reuse.
        let shop_id = match pool::tenant::resolve_tenant_from_host(&event.headers) {
            Some(id) if !id.is_empty() => id,
            _ => {
                // No tenant — not a storefront request. Drop silently.
                continue;
            }
        };

        if conn.is_none() {
            conn = connect(&redis_url);
            if conn.is_none() {
                // Redis unreachable — drop this event. We retry on the next
                // one; no tight spin because `rx.recv()` blocks until work.
                continue;
            }
        }

        let c = conn.as_mut().expect("connection checked above");
        if let Err(err) = apply_event(c, &shop_id, &event.day) {
            tracing::debug!(
                "pageview redis write failed for {}: {} — reconnecting",
                shop_id,
                err
            );
            conn = None;
        }
    }
    tracing::debug!("pageview tracker shutting down (sender dropped)");
}

fn connect(url: &str) -> Option<redis::Connection> {
    let client = match redis::Client::open(url) {
        Ok(c) => c,
        Err(err) => {
            tracing::debug!("pageview tracker: cannot open redis client: {}", err);
            return None;
        }
    };
    match client.get_connection_with_timeout(Duration::from_millis(500)) {
        Ok(c) => Some(c),
        Err(err) => {
            tracing::debug!("pageview tracker: cannot connect to redis: {}", err);
            None
        }
    }
}

/// Apply a single event: bumps the lifetime + daily counters and refreshes
/// the daily TTL. Uses a pipeline so it's one round-trip.
fn apply_event(
    conn: &mut redis::Connection,
    shop_id: &str,
    day: &str,
) -> Result<(), redis::RedisError> {
    let total_key = format!("analytics:{}:pageviews:total", shop_id);
    let daily_key = format!("analytics:{}:pageviews:{}", shop_id, day);

    redis::pipe()
        .atomic()
        .cmd("INCR")
        .arg(&total_key)
        .ignore()
        .cmd("INCR")
        .arg(&daily_key)
        .ignore()
        .cmd("EXPIRE")
        .arg(&daily_key)
        .arg(DAILY_TTL_SECS)
        .ignore()
        .query::<()>(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn should_track_plain_html_200() {
        let h = header_map(&[("Content-Type", "text/html; charset=utf-8")]);
        assert!(should_track(200, &h));
    }

    #[test]
    fn should_track_case_insensitive_header_name() {
        let h = header_map(&[("content-TYPE", "text/html")]);
        assert!(should_track(200, &h));
    }

    #[test]
    fn should_track_2xx_range() {
        let h = header_map(&[("content-type", "text/html")]);
        assert!(should_track(200, &h));
        assert!(should_track(299, &h));
        assert!(!should_track(199, &h));
        assert!(!should_track(300, &h));
    }

    #[test]
    fn should_track_skips_json() {
        let h = header_map(&[("content-type", "application/json")]);
        assert!(!should_track(200, &h));
    }

    #[test]
    fn should_track_skips_asset() {
        let h = header_map(&[("content-type", "image/png")]);
        assert!(!should_track(200, &h));
    }

    #[test]
    fn should_track_skips_redirect() {
        let h = header_map(&[("content-type", "text/html")]);
        assert!(!should_track(301, &h));
        assert!(!should_track(302, &h));
    }

    #[test]
    fn should_track_skips_error() {
        let h = header_map(&[("content-type", "text/html")]);
        assert!(!should_track(404, &h));
        assert!(!should_track(500, &h));
    }

    #[test]
    fn should_track_skips_missing_content_type() {
        let h = header_map(&[("x-other", "1")]);
        assert!(!should_track(200, &h));
    }

    #[test]
    fn should_track_skips_text_html_substring() {
        // "text/htmlthing" is NOT html — we match the exact media type.
        let h = header_map(&[("content-type", "text/htmlthing")]);
        assert!(!should_track(200, &h));
    }

    #[test]
    fn should_track_handles_params_after_semicolon() {
        let h = header_map(&[("content-type", "TEXT/HTML ; charset=UTF-8")]);
        assert!(should_track(200, &h));
    }

    #[test]
    fn should_track_accepts_single_quoted_key() {
        // The JS→Rust response envelope bridge currently ships the content
        // type header with literal single quotes in its key. We strip them
        // rather than letting the bug gate analytics.
        let h = header_map(&[("'content-type'", "text/html; charset=utf-8")]);
        assert!(should_track(200, &h));
    }

    #[test]
    fn should_track_accepts_double_quoted_key() {
        let h = header_map(&[("\"content-type\"", "text/html")]);
        assert!(should_track(200, &h));
    }

    #[test]
    fn header_name_is_content_type_rejects_mismatched_quotes() {
        assert!(!header_name_is_content_type("'content-type\""));
        assert!(!header_name_is_content_type("content-type'"));
        assert!(header_name_is_content_type("CONTENT-TYPE"));
        assert!(header_name_is_content_type("  content-type  "));
    }

    #[test]
    fn track_pageview_returns_false_on_filter_miss() {
        let h = header_map(&[("content-type", "application/json")]);
        let req_headers: Vec<(String, String)> = vec![
            ("host".to_string(), "shop_alpha.tana.gg".to_string()),
        ];
        // Filter rejects before the worker is touched.
        assert!(!track_pageview(&req_headers, 200, &h));
    }

    #[test]
    fn track_pageview_returns_false_for_non_2xx() {
        let h = header_map(&[("content-type", "text/html")]);
        let req_headers: Vec<(String, String)> = vec![
            ("host".to_string(), "shop_alpha.tana.gg".to_string()),
        ];
        assert!(!track_pageview(&req_headers, 404, &h));
        assert!(!track_pageview(&req_headers, 302, &h));
    }

    // End-to-end integration test: requires a reachable Redis at
    // DEKA_REDIS_TEST_URL. Skips silently if not configured so CI stays clean.
    //
    // Seeds a `subdomain:{name}` → `{shop_id}` key in Redis so the
    // Host-based resolver can find it (analytics now uses
    // `resolve_tenant_from_host` which ignores X-Shop-ID).
    #[test]
    fn end_to_end_redis_integration() {
        let url = match std::env::var("DEKA_REDIS_TEST_URL") {
            Ok(v) => v,
            Err(_) => return,
        };
        let client = match redis::Client::open(url.as_str()) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut probe = match client.get_connection_with_timeout(Duration::from_millis(500)) {
            Ok(c) => c,
            Err(_) => return,
        };

        // Point both the pageview worker *and* the tenant resolver at the
        // test Redis. SAFETY: tests set these before the worker spawns.
        unsafe {
            std::env::set_var("DEKA_REDIS_URL", &url);
        }
        let pid = std::process::id();
        let subdomain = format!("pvtest{}", pid);
        let shop = format!("shop_pvtest_{}", pid);
        let subdomain_key = format!("subdomain:{}", subdomain);
        let total = format!("analytics:{}:pageviews:total", shop);
        let daily = format!("analytics:{}:pageviews:{}", shop, today_utc());

        // Seed the subdomain → shop_id mapping
        let _: () = redis::cmd("SET")
            .arg(&subdomain_key)
            .arg(&shop)
            .query(&mut probe)
            .unwrap();
        let _: () = redis::cmd("DEL")
            .arg(&total)
            .arg(&daily)
            .query(&mut probe)
            .unwrap();

        let response_headers = header_map(&[("Content-Type", "text/html; charset=utf-8")]);
        let request_headers: Vec<(String, String)> = vec![
            ("Host".to_string(), format!("{}.tana.gg", subdomain)),
        ];
        assert!(track_pageview(&request_headers, 200, &response_headers));
        assert!(track_pageview(&request_headers, 200, &response_headers));
        assert!(track_pageview(&request_headers, 200, &response_headers));

        // Give the worker a moment to drain the channel.
        std::thread::sleep(Duration::from_millis(300));

        let total_count: i64 = redis::cmd("GET").arg(&total).query(&mut probe).unwrap();
        let daily_count: i64 = redis::cmd("GET").arg(&daily).query(&mut probe).unwrap();
        assert_eq!(total_count, 3);
        assert_eq!(daily_count, 3);

        let ttl: i64 = redis::cmd("TTL").arg(&daily).query(&mut probe).unwrap();
        assert!(ttl > 0, "daily key should have a TTL, got {}", ttl);

        let _: () = redis::cmd("DEL")
            .arg(&total)
            .arg(&daily)
            .arg(&subdomain_key)
            .query(&mut probe)
            .unwrap();
    }
}
