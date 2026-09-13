//! Exercise the real router, including its dev route gating and shared limiter.
use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use crate::{
    config::HttpConfig,
    rate_limit::{RateLimitConfig, RateLimiter},
    router::app_router_with_rate_limiter,
};

fn router(dev_mode: bool) -> Router {
    let engine = Arc::new(engine::RuntimeEngine::new(
        pool::PoolConfig {
            num_workers: 1,
            ..Default::default()
        },
        &engine::config::RuntimeConfig::default(),
        Arc::new(Vec::new),
    ));
    let state = Arc::new(engine::RuntimeState {
        engine,
        handler_code: String::new(),
        handler_entry: None,
        public_dir: None,
        artifact_manifest: None,
        handler_key: pool::HandlerKey::new("rate-limit-937".to_string()),
        dev_mode,
        perf_mode: false,
        perf_request_value: serde_json::Value::Null,
        security: pool::ExecutionSecurity {
            policy_json: "{}".to_string(),
            no_prompt: true,
        },
    });
    // A static directory makes unrecognized paths 404 without executing JS.
    let config = HttpConfig {
        static_entry: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor")),
        ..Default::default()
    };
    app_router_with_rate_limiter(
        state,
        Arc::new(RateLimiter::new(config.rate_limit.clone())),
        config,
    )
}

async fn status(app: &Router, path: &str, peer: u16) -> StatusCode {
    let mut request = Request::builder().uri(path).body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            peer,
        ))));
    app.clone().oneshot(request).await.unwrap().status()
}

const DEV_PATHS: [&str; 3] = [
    "/_deka/react/react-dom-client.js",
    "/_deka/hmr/module/missing-937.js",
    "/_deka/hmr",
];

#[tokio::test]
async fn dev_fan_out_bypasses_even_an_exhausted_application_bucket() {
    let app = router(true);
    let burst = RateLimitConfig::default().burst;
    for _ in 0..burst * 3 {
        for path in DEV_PATHS {
            assert_ne!(
                status(&app, path, 9000).await,
                StatusCode::TOO_MANY_REQUESTS,
                "{path}"
            );
        }
    }
    // Fan-out consumed none of the app's initial burst. Different source ports
    // still share one per-IP bucket, as real parallel browser connections do.
    for i in 0..burst {
        assert_eq!(
            status(&app, "/missing-app", 10000 + i as u16).await,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        status(&app, "/missing-app", 11000).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    for path in DEV_PATHS {
        assert_ne!(
            status(&app, path, 12000).await,
            StatusCode::TOO_MANY_REQUESTS,
            "{path}"
        );
    }
    // Prefix lookalikes must not acquire the exemption.
    assert_eq!(
        status(&app, "/_deka-other/file.js", 12000).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn production_dev_paths_stay_unavailable_and_rate_limited() {
    let app = router(false);
    let burst = RateLimitConfig::default().burst;
    for i in 0..burst {
        assert_eq!(
            status(&app, DEV_PATHS[i as usize % DEV_PATHS.len()], 9000).await,
            StatusCode::NOT_FOUND
        );
    }
    for path in DEV_PATHS {
        assert_eq!(
            status(&app, path, 9000).await,
            StatusCode::TOO_MANY_REQUESTS,
            "{path}"
        );
    }
}
