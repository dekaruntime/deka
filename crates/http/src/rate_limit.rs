use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use dashmap::DashMap;
use serde_json::json;

const DEFAULT_REQUESTS_PER_MINUTE: u32 = 600;
const DEFAULT_BURST: u32 = 60;
const JANITOR_INTERVAL: Duration = Duration::from_secs(5 * 60);
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
    pub burst: u32,
    pub disabled: bool,
}

impl RateLimitConfig {
    pub fn from_env() -> Self {
        Self {
            requests_per_minute: parse_env_u32(
                "DEKA_RATE_LIMIT_REQUESTS_PER_MINUTE",
                DEFAULT_REQUESTS_PER_MINUTE,
            ),
            burst: parse_env_u32("DEKA_RATE_LIMIT_BURST", DEFAULT_BURST),
            disabled: std::env::var("DEKA_RATE_LIMIT_DISABLED")
                .map(|value| is_truthy(&value))
                .unwrap_or(false),
        }
    }

    fn refill_per_second(&self) -> f64 {
        f64::from(self.requests_per_minute) / 60.0
    }
}

#[derive(Debug)]
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: DashMap<IpAddr, TokenBucket>,
}

impl RateLimiter {
    pub fn from_env() -> Arc<Self> {
        Arc::new(Self::new(RateLimitConfig::from_env()))
    }

    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: DashMap::new(),
        }
    }

    pub fn spawn_janitor(self: &Arc<Self>) {
        if self.config.disabled {
            return;
        }

        let limiter = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(JANITOR_INTERVAL);
            loop {
                interval.tick().await;
                limiter.remove_stale(Instant::now(), STALE_AFTER);
            }
        });
    }

    pub fn check(&self, ip: IpAddr) -> RateLimitDecision {
        self.check_at(ip, Instant::now())
    }

    fn check_at(&self, ip: IpAddr, now: Instant) -> RateLimitDecision {
        if self.config.disabled {
            return RateLimitDecision::Allowed;
        }

        let burst = f64::from(self.config.burst);
        let refill_per_second = self.config.refill_per_second();
        let mut bucket = self
            .buckets
            .entry(ip)
            .or_insert_with(|| TokenBucket::new(burst, now));

        bucket.check(now, burst, refill_per_second)
    }

    fn remove_stale(&self, now: Instant, stale_after: Duration) {
        self.buckets
            .retain(|_, bucket| now.duration_since(bucket.last_seen) < stale_after);
    }

    #[cfg(test)]
    fn contains_ip(&self, ip: IpAddr) -> bool {
        self.buckets.contains_key(&ip)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allowed,
    Limited { retry_after_secs: u64 },
}

#[derive(Clone, Debug)]
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
    last_seen: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, now: Instant) -> Self {
        Self {
            tokens: capacity,
            last_refill: now,
            last_seen: now,
        }
    }

    fn check(&mut self, now: Instant, capacity: f64, refill_per_second: f64) -> RateLimitDecision {
        self.refill(now, capacity, refill_per_second);
        self.last_seen = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            RateLimitDecision::Allowed
        } else {
            RateLimitDecision::Limited {
                retry_after_secs: retry_after_secs(self.tokens, refill_per_second),
            }
        }
    }

    fn refill(&mut self, now: Instant, capacity: f64, refill_per_second: f64) {
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        if elapsed <= 0.0 {
            return;
        }

        self.tokens = (self.tokens + (elapsed * refill_per_second)).min(capacity);
        self.last_refill = now;
    }
}

pub async fn middleware(
    State(limiter): State<Arc<RateLimiter>>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(ip) = source_ip(request.headers(), connect_info.as_ref().map(|info| info.0)) else {
        return next.run(request).await;
    };

    match limiter.check(ip) {
        RateLimitDecision::Allowed => next.run(request).await,
        RateLimitDecision::Limited { retry_after_secs } => rate_limited_response(retry_after_secs),
    }
}

pub(crate) fn source_ip(headers: &HeaderMap, peer_addr: Option<SocketAddr>) -> Option<IpAddr> {
    header_ip(headers, "cf-connecting-ip")
        .or_else(|| x_forwarded_for_ip(headers))
        .or_else(|| peer_addr.map(|addr| addr.ip()))
}

fn header_ip(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
}

fn x_forwarded_for_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse().ok())
}

fn rate_limited_response(retry_after_secs: u64) -> Response {
    let body = json!({
        "error": "rate_limited",
        "retry_after": retry_after_secs,
    })
    .to_string();

    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header(RETRY_AFTER, retry_after_secs.to_string())
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn retry_after_secs(tokens: f64, refill_per_second: f64) -> u64 {
    if refill_per_second <= 0.0 {
        return 60;
    }

    ((1.0 - tokens).max(0.0) / refill_per_second)
        .ceil()
        .max(1.0) as u64
}

fn parse_env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn is_truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "yes" | "on")
}

#[cfg(test)]
mod tests {
    use super::{
        RateLimitConfig, RateLimitDecision, RateLimiter, STALE_AFTER, TokenBucket, source_ip,
    };
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tower::ServiceExt;

    use crate::rate_limit::middleware;

    #[test]
    fn token_bucket_enforces_burst_cap() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new(2.0, now);

        assert_eq!(bucket.check(now, 2.0, 1.0), RateLimitDecision::Allowed);
        assert_eq!(bucket.check(now, 2.0, 1.0), RateLimitDecision::Allowed);
        assert_eq!(
            bucket.check(now, 2.0, 1.0),
            RateLimitDecision::Limited {
                retry_after_secs: 1
            }
        );
    }

    #[test]
    fn token_bucket_refills_over_time_without_exceeding_cap() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new(3.0, now);

        for _ in 0..3 {
            assert_eq!(bucket.check(now, 3.0, 2.0), RateLimitDecision::Allowed);
        }
        assert_eq!(
            bucket.check(now, 3.0, 2.0),
            RateLimitDecision::Limited {
                retry_after_secs: 1
            }
        );

        assert_eq!(
            bucket.check(now + Duration::from_millis(500), 3.0, 2.0),
            RateLimitDecision::Allowed
        );
        bucket.refill(now + Duration::from_secs(10), 3.0, 2.0);
        assert_eq!(bucket.tokens, 3.0);
    }

    #[test]
    fn source_ip_prefers_cloudflare_then_forwarded_then_peer() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 1234));
        let request = Request::builder()
            .header("x-forwarded-for", "198.51.100.20, 10.0.0.1")
            .header("cf-connecting-ip", "203.0.113.9")
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            source_ip(request.headers(), Some(peer)),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)))
        );

        let request = Request::builder()
            .header("x-forwarded-for", "198.51.100.20, 10.0.0.1")
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            source_ip(request.headers(), Some(peer)),
            Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 20)))
        );

        let request = Request::builder().body(Body::empty()).unwrap();
        assert_eq!(
            source_ip(request.headers(), Some(peer)),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
    }

    #[test]
    fn janitor_removes_stale_entries() {
        let limiter = RateLimiter::new(test_config(60, 1, false));
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 44));
        let now = Instant::now();

        assert_eq!(limiter.check_at(ip, now), RateLimitDecision::Allowed);
        limiter.remove_stale(now + STALE_AFTER + Duration::from_secs(1), STALE_AFTER);

        assert!(!limiter.contains_ip(ip));
    }

    #[tokio::test]
    async fn middleware_returns_429_with_retry_after_after_burst() {
        let limiter = Arc::new(RateLimiter::new(test_config(60, 2, false)));
        let app = test_app(Arc::clone(&limiter));

        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(request_from("203.0.113.10"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = app.oneshot(request_from("203.0.113.10")).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()["retry-after"], "1");

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            body.as_ref(),
            br#"{"error":"rate_limited","retry_after":1}"#
        );
    }

    #[tokio::test]
    async fn middleware_uses_cf_connecting_ip_as_key() {
        let limiter = Arc::new(RateLimiter::new(test_config(60, 5, false)));
        let app = test_app(Arc::clone(&limiter));
        let cf_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 88));

        let response = app.oneshot(request_from("203.0.113.88")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(limiter.contains_ip(cf_ip));
        assert!(!limiter.contains_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }

    #[tokio::test]
    async fn disabled_mode_allows_all_requests() {
        let limiter = Arc::new(RateLimiter::new(test_config(60, 1, true)));
        let app = test_app(limiter);

        for _ in 0..5 {
            let response = app
                .clone()
                .oneshot(request_from("203.0.113.77"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    fn test_app(limiter: Arc<RateLimiter>) -> Router {
        Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(from_fn_with_state(limiter, middleware))
    }

    fn request_from(ip: &str) -> Request<Body> {
        Request::builder()
            .uri("/")
            .header("cf-connecting-ip", ip)
            .body(Body::empty())
            .unwrap()
    }

    fn test_config(requests_per_minute: u32, burst: u32, disabled: bool) -> RateLimitConfig {
        RateLimitConfig {
            requests_per_minute,
            burst,
            disabled,
        }
    }
}
