//! `deka platform` — multi-tenant serve mode.
//!
//! Each tenant gets their own PHPX handler from `tenants/{shop_id}/main.phpx`.
//! Falls back to `default/main.phpx` if the tenant dir doesn't exist.
//! Uses the bundler (same as `deka serve`) to compile PHPX→JS with stdlib prelude.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::response::{IntoResponse, Response};
use axum::Router;
use core::Context;
use engine::config as runtime_config;
use engine::{RuntimeEngine, set_engine};
use pool::{ExecutionMode, HandlerKey, PoolConfig, RequestData, RequestParts};

use crate::js_pipeline::build_phpx_handler_bundle;

pub fn platform(context: &Context) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
    rt.block_on(platform_async(context));
}

/// Platform state shared across all requests.
struct PlatformState {
    engine: Arc<RuntimeEngine>,
    root: PathBuf,
    /// Cache of bundled handler code per tenant (shop_id → compiled JS).
    bundle_cache: Mutex<HashMap<String, String>>,
}

impl PlatformState {
    /// Returns (HandlerKey, handler_code, handler_entry) for a tenant.
    /// Uses the bundler to compile PHPX→JS with stdlib prelude baked in.
    /// Caches the result so subsequent requests are fast.
    fn resolve_handler(&self, shop_id: &str) -> (HandlerKey, String, Option<String>) {
        let cache_key = if shop_id.is_empty() { "default" } else { shop_id };

        // Check cache first
        {
            let cache = self.bundle_cache.lock().unwrap();
            if let Some(code) = cache.get(cache_key) {
                return (
                    HandlerKey::new(if shop_id.is_empty() { "default".to_string() } else { format!("tenant:{}", shop_id) }),
                    code.clone(),
                    None, // No entry needed — code is already bundled
                );
            }
        }

        // Resolve handler path — try tenant-specific, fall back to default
        let handler_path = if shop_id.is_empty() {
            self.root.join("default").join("main.phpx")
        } else {
            let tenant = self.root.join("tenants").join(shop_id).join("main.phpx");
            if tenant.exists() { tenant } else { self.root.join("default").join("main.phpx") }
        };

        // Bundle using the same pipeline as `deka serve`.
        // If the tenant-specific bundle fails (e.g. missing stdlib.json),
        // fall back to the default handler so we never serve empty code.
        let handler_str = handler_path.to_string_lossy().to_string();
        let code = match build_phpx_handler_bundle(&handler_str) {
            Ok(bundled) => {
                stdio::log("platform", &format!("bundled {} ({} bytes)", cache_key, bundled.len()));
                bundled
            }
            Err(err) => {
                stdio::error("platform", &format!("bundle failed for {}: {}", cache_key, err));
                // Fall back to the default handler — never serve empty code
                let default_path = self.root.join("default").join("main.phpx");
                let default_str = default_path.to_string_lossy().to_string();
                if handler_path != default_path {
                    match build_phpx_handler_bundle(&default_str) {
                        Ok(bundled) => {
                            stdio::log("platform", &format!(
                                "fallback to default for {} ({} bytes)", cache_key, bundled.len()
                            ));
                            bundled
                        }
                        Err(err2) => {
                            stdio::error("platform", &format!(
                                "default fallback also failed for {}: {}", cache_key, err2
                            ));
                            String::new()
                        }
                    }
                } else {
                    String::new()
                }
            }
        };

        // Cache the bundled code
        {
            let mut cache = self.bundle_cache.lock().unwrap();
            cache.insert(cache_key.to_string(), code.clone());
        }

        (
            HandlerKey::new(if shop_id.is_empty() { "default".to_string() } else { format!("tenant:{}", shop_id) }),
            code,
            None, // Bundled code — no ESM entry needed
        )
    }
}

async fn platform_async(context: &Context) {
    crate::env::init_env();

    let input = &context.handler.input;
    let root = PathBuf::from(if input.is_empty() { "." } else { input });
    let root = std::fs::canonicalize(&root).unwrap_or(root);

    // Load database config from platform-level deka.json
    runtime_config::load_database_config(&root);

    // Set security env vars (permissive defaults for platform mode)
    unsafe {
        std::env::set_var("DEKA_SECURITY_ENFORCE", "1");
        std::env::set_var("DEKA_SECURITY_NO_PROMPT", "1");
        std::env::set_var("PHPX_MODULE_ROOT", root.join("default").to_string_lossy().as_ref());
    }

    // Validate directory structure
    let default_dir = root.join("default");
    let tenants_dir = root.join("tenants");
    let default_handler = default_dir.join("main.phpx");

    if !default_handler.exists() {
        stdio::error("platform", &format!(
            "missing default/main.phpx at {}", default_dir.display()
        ));
        std::process::exit(1);
    }

    if !tenants_dir.exists() {
        let _ = std::fs::create_dir_all(&tenants_dir);
    }

    // Pre-bundle the default handler to catch errors early
    let default_handler_str = default_handler.to_string_lossy().to_string();
    match build_phpx_handler_bundle(&default_handler_str) {
        Ok(bundled) => {
            stdio::log("platform", &format!("default handler bundled ({} bytes)", bundled.len()));
        }
        Err(err) => {
            stdio::error("platform", &format!("failed to bundle default handler: {}", err));
            std::process::exit(1);
        }
    }

    let tenant_count = std::fs::read_dir(&tenants_dir)
        .map(|entries| entries.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count())
        .unwrap_or(0);

    stdio::log("platform", &format!(
        "root: {}, default: ok, tenants: {}", root.display(), tenant_count
    ));

    // Build pool config
    let pool_config = PoolConfig::from_env();
    let user_pool_config = PoolConfig {
        num_workers: 1,
        ..PoolConfig::default()
    };

    let serve_mode = runtime_config::ServeMode::Php;
    let extensions_provider = Arc::new(move || crate::extensions::extensions_for_mode(&serve_mode));

    let runtime_cfg = runtime_config::RuntimeConfig::load();
    let engine = Arc::new(RuntimeEngine::new(
        pool_config,
        user_pool_config,
        &runtime_cfg,
        extensions_provider,
    ));
    let _ = set_engine(Arc::clone(&engine));

    let state = Arc::new(PlatformState {
        engine,
        root: root.clone(),
        bundle_cache: Mutex::new(HashMap::new()),
    });

    // Determine port
    let port: u16 = context
        .args
        .params
        .get("--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(8530);

    let listener = match TcpListener::bind(format!("127.0.0.1:{}", port)) {
        Ok(l) => l,
        Err(err) => {
            stdio::error("platform", &format!("failed to bind port {}: {}", port, err));
            std::process::exit(1);
        }
    };
    listener.set_nonblocking(true).ok();

    stdio::log("listen", &format!("http://localhost:{}", port));

    let app = Router::new()
        .fallback(handle_platform_request)
        .with_state(state);

    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_platform_request(
    State(state): State<Arc<PlatformState>>,
    request: Request,
) -> impl IntoResponse {
    let method = request.method().as_str().to_string();
    let uri = request.uri().to_string();

    // Extract headers
    let mut headers = Vec::with_capacity(request.headers().len());
    for (key, value) in request.headers().iter() {
        headers.push((
            key.as_str().to_string(),
            value.to_str().unwrap_or("").to_string(),
        ));
    }

    // Read body
    let content_len = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(usize::MAX);

    let body = if content_len == 0 {
        None
    } else {
        match axum::body::to_bytes(request.into_body(), usize::MAX).await {
            Ok(bytes) if !bytes.is_empty() => Some(String::from_utf8_lossy(&bytes).to_string()),
            _ => None,
        }
    };

    // Strip X-Shop-ID from untrusted external requests — in platform
    // (multi-tenant) mode only the Host header determines the tenant.
    // This prevents spoofed X-Shop-ID from influencing handler routing,
    // $_SERVER['SHOP_ID'] injection, and analytics attribution.
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case("x-shop-id"));

    // Keep a lightweight clone of the request headers for the pageview
    // tracker — it needs them to resolve the shop_id on the worker thread.
    let request_headers_for_analytics = headers.clone();

    // Resolve tenant from Host header only.
    let shop_id = pool::tenant::resolve_tenant_from_host(&headers).unwrap_or_default();
    let (handler_key, handler_code, handler_entry) = state.resolve_handler(&shop_id);

    let request_parts = RequestParts {
        url: format!("http://localhost{}", uri),
        method,
        headers,
        body,
    };

    let request_data = RequestData {
        handler_code,
        handler_entry,
        request_value: serde_json::Value::Null,
        request_parts: Some(request_parts),
        mode: ExecutionMode::Request,
    };

    match state.engine.execute(handler_key, request_data).await {
        Ok(pool_response) => {
            if !pool_response.success {
                let err = pool_response.error.unwrap_or_else(|| "Unknown error".to_string());
                return Response::builder()
                    .status(500)
                    .body(axum::body::Body::from(format!("Handler error: {}", err)))
                    .unwrap();
            }
            match pool_response.result {
                Some(result) => match engine::ResponseEnvelope::from_value(result) {
                    Ok(envelope) => {
                        // Fire-and-forget pageview tracking. Filters to
                        // 2xx + text/html inside `track_pageview`, resolves
                        // shop_id on a dedicated worker thread, writes to
                        // Redis out-of-band. Never blocks or fails the
                        // request path.
                        deka_http::analytics::track_pageview(
                            &request_headers_for_analytics,
                            envelope.status,
                            &envelope.headers,
                        );
                        let mut response = Response::builder().status(envelope.status);
                        for (key, value) in &envelope.headers {
                            response = response.header(key.as_str(), value.as_str());
                        }
                        let body_bytes = if let Some(b64) = &envelope.body_base64 {
                            use base64::Engine;
                            base64::engine::general_purpose::STANDARD
                                .decode(b64)
                                .unwrap_or_default()
                        } else {
                            envelope.body.into_bytes()
                        };
                        response
                            .body(axum::body::Body::from(body_bytes))
                            .unwrap()
                    }
                    Err(err) => Response::builder()
                        .status(500)
                        .body(axum::body::Body::from(format!("Response error: {}", err)))
                        .unwrap(),
                },
                None => Response::builder()
                    .status(500)
                    .body(axum::body::Body::from("No response from handler"))
                    .unwrap(),
            }
        }
        Err(err) => Response::builder()
            .status(500)
            .body(axum::body::Body::from(format!("Handler execution failed: {}", err)))
            .unwrap(),
    }
}

mod stdio {
    pub fn log(category: &str, message: &str) {
        eprintln!("[{}] {}", category, message);
    }
    pub fn error(category: &str, message: &str) {
        eprintln!("[{}] ERROR: {}", category, message);
    }
}
