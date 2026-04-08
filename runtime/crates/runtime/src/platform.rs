//! `deka platform` — multi-tenant serve mode.
//!
//! Each tenant gets their own PHPX handler from `tenants/{shop_id}/main.phpx`.
//! Falls back to `default/main.phpx` if the tenant dir doesn't exist.

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

use crate::extensions::extensions_for_mode;
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
}

impl PlatformState {
    /// Returns (HandlerKey, handler_code, handler_entry) for a tenant.
    /// In ESM mode, handler_code is empty and handler_entry is the file path.
    fn resolve_handler(&self, shop_id: &str) -> (HandlerKey, String, Option<String>) {
        if shop_id.is_empty() {
            let entry = self.root.join("default").join("main.phpx");
            return (
                HandlerKey::new("default"),
                String::new(),
                Some(entry.to_string_lossy().to_string()),
            );
        }

        // Check for tenant-specific handler
        let tenant_handler = self.root.join("tenants").join(shop_id).join("main.phpx");
        let entry = if tenant_handler.exists() {
            tenant_handler
        } else {
            // Fall back to default template
            self.root.join("default").join("main.phpx")
        };

        (
            HandlerKey::new(format!("tenant:{}", shop_id)),
            String::new(),
            Some(entry.to_string_lossy().to_string()),
        )
    }
}

async fn platform_async(context: &Context) {
    let input = &context.handler.input;
    let root = PathBuf::from(if input.is_empty() { "." } else { input });
    let root = std::fs::canonicalize(&root).unwrap_or(root);

    // Load database config
    runtime_config::load_database_config(&root);

    // Set PHPX module root
    unsafe {
        std::env::set_var("PHPX_MODULE_ROOT", root.to_string_lossy().as_ref());
    }

    // Validate directory structure
    let default_dir = root.join("default");
    let tenants_dir = root.join("tenants");
    let default_handler = default_dir.join("main.phpx");

    if !default_handler.exists() {
        stdio::error("platform", &format!(
            "missing default/main.phpx at {}", default_dir.display()
        ));
        stdio::log("platform", "expected directory structure:");
        stdio::log("platform", "  default/main.phpx    (template for new tenants)");
        stdio::log("platform", "  tenants/             (per-tenant overrides)");
        std::process::exit(1);
    }

    if !tenants_dir.exists() {
        let _ = std::fs::create_dir_all(&tenants_dir);
    }

    // In ESM mode (default), handler_code is empty — the ESM loader resolves modules dynamically.
    // We just store the handler file path as the entry point.
    let default_code = String::new();

    // List existing tenants
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
    let extensions_provider = Arc::new(move || extensions_for_mode(&serve_mode));

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

    // Resolve tenant
    let shop_id = pool::tenant::resolve_tenant_from_headers(&headers).unwrap_or_default();
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

fn load_and_compile_handler(path: &Path) -> Result<String, String> {
    let path_str = path.to_string_lossy().to_string();
    build_phpx_handler_bundle(&path_str)
}

mod stdio {
    pub fn log(category: &str, message: &str) {
        eprintln!("[{}] {}", category, message);
    }
    pub fn error(category: &str, message: &str) {
        eprintln!("[{}] ERROR: {}", category, message);
    }
}
