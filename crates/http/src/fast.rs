use std::net::SocketAddr;
use std::sync::Arc;

use base64::Engine;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperBuilder;
use serde_json::json;

use engine::{RuntimeState, execute_request_parts};

use crate::rate_limit::{RateLimitDecision, RateLimiter, source_ip};

pub async fn serve_http_fast(
    listener: tokio::net::TcpListener,
    state: Arc<RuntimeState>,
    rate_limiter: Arc<RateLimiter>,
    debug: bool,
) {
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!("HTTP accept failed: {}", err);
                continue;
            }
        };
        let state = Arc::clone(&state);
        let rate_limiter = Arc::clone(&rate_limiter);
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                handle_request_fast(
                    Arc::clone(&state),
                    Arc::clone(&rate_limiter),
                    peer_addr,
                    debug,
                    req,
                )
            });
            let builder = HyperBuilder::new(TokioExecutor::new());
            if let Err(err) = builder.serve_connection(io, service).await {
                tracing::warn!("HTTP connection failed: {}", err);
            }
        });
    }
}

async fn handle_request_fast(
    state: Arc<RuntimeState>,
    rate_limiter: Arc<RateLimiter>,
    peer_addr: SocketAddr,
    debug: bool,
    request: hyper::Request<Incoming>,
) -> Result<hyper::Response<Full<Bytes>>, hyper::Error> {
    let method = request.method().as_str().to_string();
    let uri = request.uri().to_string();
    if debug {
        tracing::info!("[http-fast] request {} {}", method, uri);
    }
    if let Some(response) = fast_rate_limit_response(&rate_limiter, request.headers(), peer_addr) {
        return Ok(response);
    }
    let _ = request.into_body();

    let response = match execute_request_parts(
        Arc::clone(&state),
        format!("http://localhost{}", uri),
        method,
        Vec::new(),
        None,
    )
    .await
    {
        Ok(response_envelope) => response_envelope,
        Err(err) => {
            tracing::error!("Handler execution failed: {}", err);
            let response = hyper::Response::builder().status(500);
            let body = Full::new(Bytes::from(handler_failure_body(&err, state.dev_mode)));
            return Ok(response.body(body).unwrap());
        }
    };
    if debug {
        tracing::info!("[http-fast] response {} {}", response.status, uri);
    }

    let mut builder = hyper::Response::builder().status(response.status);
    for (key, value) in response.headers {
        if key.eq_ignore_ascii_case("set-cookie") && value.contains('\n') {
            for part in value.split('\n').filter(|part| !part.is_empty()) {
                builder = builder.header(&key, part);
            }
            continue;
        }
        builder = builder.header(&key, value);
    }

    let body = if let Some(body_base64) = response.body_base64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(body_base64.as_bytes())
            .unwrap_or_default();
        Full::new(Bytes::from(bytes))
    } else {
        Full::new(Bytes::from(response.body))
    };

    Ok(builder.body(body).unwrap())
}

fn handler_failure_body(detail: &str, dev_mode: bool) -> String {
    if dev_mode {
        format!("Handler execution failed: {}", detail)
    } else {
        "Internal Server Error".to_string()
    }
}

fn rate_limited_response_fast(retry_after_secs: u64) -> hyper::Response<Full<Bytes>> {
    let body = json!({
        "error": "rate_limited",
        "retry_after": retry_after_secs,
    })
    .to_string();

    hyper::Response::builder()
        .status(429)
        .header("retry-after", retry_after_secs.to_string())
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

fn fast_rate_limit_response(
    rate_limiter: &RateLimiter,
    headers: &hyper::HeaderMap,
    peer_addr: SocketAddr,
) -> Option<hyper::Response<Full<Bytes>>> {
    let ip = source_ip(headers, Some(peer_addr))?;
    match rate_limiter.check(ip) {
        RateLimitDecision::Allowed => None,
        RateLimitDecision::Limited { retry_after_secs } => {
            Some(rate_limited_response_fast(retry_after_secs))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fast_rate_limit_response;
    use crate::rate_limit::{RateLimitConfig, RateLimiter};
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::Request;
    use std::net::SocketAddr;

    #[test]
    fn fast_path_uses_shared_rate_limiter() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_minute: 60,
            burst: 1,
            disabled: false,
        });
        let peer = SocketAddr::from(([127, 0, 0, 1], 1234));
        let request = Request::builder()
            .header("cf-connecting-ip", "203.0.113.55")
            .body(Full::new(Bytes::new()))
            .unwrap();

        assert!(fast_rate_limit_response(&limiter, request.headers(), peer).is_none());

        let response = fast_rate_limit_response(&limiter, request.headers(), peer)
            .expect("second request should be rate limited");
        assert_eq!(response.status(), 429);
        assert_eq!(response.headers()["retry-after"], "1");
    }
}
