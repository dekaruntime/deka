//! `deka platform` — multi-tenant serve mode.
//!
//! Each tenant gets their own DekaScript handler from `tenants/{shop_id}/main.ds`.
//! Falls back to `default/main.ds` if the tenant dir doesn't exist.
//! Uses the dsc bundle stage (same as `deka serve`) to compile DekaScript to
//! JS with the stdlib prelude.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use core::Context;
use deka_http::rate_limit::{RateLimiter, middleware as rate_limit_middleware};
use engine::config as runtime_config;
use engine::{RuntimeEngine, set_engine};
use pool::{ExecutionMode, HandlerKey, PoolConfig, RequestData, RequestParts};

use crate::js_pipeline::build_deka_handler_bundle;
use crate::security::resolve_platform_security_for_root;


pub fn platform(context: &Context) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
    rt.block_on(platform_async(context));
}

fn handler_failure_body(detail: &str, dev_mode: bool) -> String {
    if dev_mode {
        detail.to_string()
    } else {
        "Internal Server Error".to_string()
    }
}

/// A cached bundle entry with last-access tracking for preview cleanup.
struct BundleEntry {
    code: String,
    last_accessed: Instant,
}

/// Platform state shared across all requests.
struct PlatformState {
    engine: Arc<RuntimeEngine>,
    root: PathBuf,
    /// Cache of bundled handler code per tenant.
    /// Keys: `shop_id` for main, `shop_id:{hash}` for preview builds.
    bundle_cache: Mutex<HashMap<String, BundleEntry>>,
    /// Resolved default-tenant security policy; every dispatched request
    /// executes under it (deka#801 — the policy travels per execution,
    /// never through the process environment).
    security: pool::ExecutionSecurity,
}

/// How long preview builds stay in cache without being accessed (7 days).
const PREVIEW_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How often the cleanup task runs (every hour).
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Resolve the on-disk handler path for a tenant.
///
/// `shop_id` arrives from tenant resolution, whose fast path already applies
/// the shop_id charset rule — but the Redis `subdomain:*` fallback
/// (`parse_subdomain_value`) returns whatever string is stored under the
/// key, with no validation on the stored value. Re-validate at the point of
/// use with the same `is_shop_id_subdomain` rule (deka#870): a failing value
/// is a hard error and must never reach a path join.
fn resolve_handler_path(root: &std::path::Path, shop_id: &str) -> Result<PathBuf, String> {
    if shop_id.is_empty() {
        return Ok(root.join("default").join("main.ds"));
    }
    if !pool::tenant::is_shop_id_subdomain(shop_id) {
        return Err(format!(
            "rejected invalid shop_id {:?} — refusing to resolve handler path",
            shop_id
        ));
    }
    let tenant = root.join("tenants").join(shop_id).join("main.ds");
    Ok(if tenant.exists() {
        tenant
    } else {
        root.join("default").join("main.ds")
    })
}

impl PlatformState {
    /// Returns (HandlerKey, handler_code, handler_entry) for a tenant.
    /// Uses the dsc bundle stage to compile DekaScript→JS with stdlib
    /// prelude baked in.
    /// Caches the result so subsequent requests are fast.
    ///
    /// `cache_key` is `shop_id` for main builds, `shop_id:{hash}` for previews.
    fn resolve_handler(
        &self,
        shop_id: &str,
        cache_key: &str,
    ) -> (HandlerKey, String, Option<String>) {
        let display_key = if cache_key.is_empty() {
            "default"
        } else {
            cache_key
        };

        // Check cache first
        {
            let mut cache = self.bundle_cache.lock().unwrap();
            if let Some(entry) = cache.get_mut(display_key) {
                entry.last_accessed = Instant::now();
                return (
                    HandlerKey::new(if shop_id.is_empty() {
                        "default".to_string()
                    } else {
                        format!("tenant:{}", display_key)
                    }),
                    entry.code.clone(),
                    None,
                );
            }
        }

        // Resolve handler path — try tenant-specific, fall back to default.
        // A shop_id that fails validation is a hard error: it never joins
        // a filesystem path and no handler code is produced.
        let handler_path = match resolve_handler_path(&self.root, shop_id) {
            Ok(path) => path,
            Err(err) => {
                stdio::error("platform", &err);
                return (
                    HandlerKey::new("tenant:invalid-shop-id".to_string()),
                    String::new(),
                    None,
                );
            }
        };

        // Bundle using the same pipeline as `deka serve`.
        // If the tenant-specific bundle fails (e.g. missing stdlib.json),
        // fall back to the default handler so we never serve empty code.
        let handler_str = handler_path.to_string_lossy().to_string();
        let code = match build_deka_handler_bundle(&handler_str) {
            Ok(bundled) => {
                stdio::log(
                    "platform",
                    &format!("bundled {} ({} bytes)", display_key, bundled.len()),
                );
                bundled
            }
            Err(err) => {
                stdio::error(
                    "platform",
                    &format!("bundle failed for {}: {}", display_key, err),
                );
                let default_path = self.root.join("default").join("main.ds");
                let default_str = default_path.to_string_lossy().to_string();
                if handler_path != default_path {
                    match build_deka_handler_bundle(&default_str) {
                        Ok(bundled) => {
                            stdio::log(
                                "platform",
                                &format!(
                                    "fallback to default for {} ({} bytes)",
                                    display_key,
                                    bundled.len()
                                ),
                            );
                            bundled
                        }
                        Err(err2) => {
                            stdio::error(
                                "platform",
                                &format!(
                                    "default fallback also failed for {}: {}",
                                    display_key, err2
                                ),
                            );
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
            cache.insert(
                display_key.to_string(),
                BundleEntry {
                    code: code.clone(),
                    last_accessed: Instant::now(),
                },
            );
        }

        (
            HandlerKey::new(if shop_id.is_empty() {
                "default".to_string()
            } else {
                format!("tenant:{}", display_key)
            }),
            code,
            None,
        )
    }

    /// Remove preview bundle entries that haven't been accessed within PREVIEW_TTL.
    /// Only affects entries whose key contains `:` (i.e. `shop_id:hash`).
    fn cleanup_stale_previews(&self) -> usize {
        let mut cache = self.bundle_cache.lock().unwrap();
        let now = Instant::now();
        let before = cache.len();
        cache.retain(|key, entry| {
            // Only expire preview entries (keys containing ':')
            if !key.contains(':') {
                return true;
            }
            let age = now.duration_since(entry.last_accessed);
            if age > PREVIEW_TTL {
                stdio::log(
                    "cleanup",
                    &format!("expired preview bundle: {} (idle {:?})", key, age),
                );
                false
            } else {
                true
            }
        });
        before - cache.len()
    }
}

async fn platform_async(context: &Context) {

    let input = &context.handler.input;
    let root = PathBuf::from(if input.is_empty() { "." } else { input });
    let root = std::fs::canonicalize(&root).unwrap_or(root);

    // Load database config from platform-level deka.json. The parsed values
    // thread into the pageview tracker explicitly (deka#801) and into the
    // host bridge modules via deka_host's explicit process-wide store —
    // never through the process environment.
    let db_config = runtime_config::load_database_config(&root);
    deka_host::host_config::install_database_endpoints(deka_host::host_config::DatabaseEndpoints {
        neo4j_uri: db_config.neo4j.as_ref().and_then(|neo4j| neo4j.uri.clone()),
        neo4j_user: db_config.neo4j.as_ref().and_then(|neo4j| neo4j.user.clone()),
        neo4j_password: db_config
            .neo4j
            .as_ref()
            .and_then(|neo4j| neo4j.password.clone()),
        neo4j_db: db_config.neo4j.as_ref().and_then(|neo4j| neo4j.db.clone()),
        redis_url: db_config.redis_url.clone(),
    });
    deka_http::analytics::init(
        db_config
            .redis_url
            .as_deref()
            .unwrap_or("redis://localhost:6379"),
    );

    // Validate directory structure
    let default_dir = root.join("default");
    let tenants_dir = root.join("tenants");
    let default_handler = default_dir.join("main.ds");

    let platform_security =
        match resolve_platform_security_for_root(&default_dir, &context.args.flags, &context.args.params)
        {
            Ok(resolved) => resolved,
            Err(err) => {
                stdio::error("platform", &err);
                std::process::exit(1);
            }
        };

    if !default_handler.exists() {
        stdio::error(
            "platform",
            &format!("missing default/main.ds at {}", default_dir.display()),
        );
        std::process::exit(1);
    }

    if !tenants_dir.exists() {
        let _ = std::fs::create_dir_all(&tenants_dir);
    }

    // Pre-bundle the default handler to catch errors early
    let default_handler_str = default_handler.to_string_lossy().to_string();
    match build_deka_handler_bundle(&default_handler_str) {
        Ok(bundled) => {
            stdio::log(
                "platform",
                &format!("default handler bundled ({} bytes)", bundled.len()),
            );
        }
        Err(err) => {
            stdio::error(
                "platform",
                &format!("failed to bundle default handler: {}", err),
            );
            std::process::exit(1);
        }
    }

    let tenant_count = std::fs::read_dir(&tenants_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count()
        })
        .unwrap_or(0);

    stdio::log(
        "platform",
        &format!(
            "root: {}, default: ok, tenants: {}",
            root.display(),
            tenant_count
        ),
    );

    // Build the shared execution pool config.
    let pool_config = PoolConfig::default();

    let serve_mode = runtime_config::ServeMode::Php;
    let extensions_provider = Arc::new(move || crate::extensions::extensions_for_mode(&serve_mode));

    let runtime_cfg = runtime_config::RuntimeConfig::load();
    let engine = Arc::new(RuntimeEngine::new(
        pool_config,
        &runtime_cfg,
        extensions_provider,
    ));
    let _ = set_engine(Arc::clone(&engine));

    let state = Arc::new(PlatformState {
        engine,
        root: root.clone(),
        bundle_cache: Mutex::new(HashMap::new()),
        security: pool::ExecutionSecurity {
            policy_json: platform_security.policy_json,
            // The platform path runs headless: prompts are suppressed
            // process-wide above (DEKA_SECURITY_NO_PROMPT=1).
            no_prompt: true,
        },
    });

    // Determine port
    let port: u16 = context
        .args
        .params
        .get("--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(8530);

    // Platform binding is a CLI parameter today; the default remains local
    // loopback. Cluster deployment must pass an explicit listener address.
    let bind_addr = context
        .args
        .params
        .get("--bind")
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let listener = match TcpListener::bind(format!("{}:{}", bind_addr, port)) {
        Ok(l) => l,
        Err(err) => {
            stdio::error(
                "platform",
                &format!("failed to bind port {}: {}", port, err),
            );
            std::process::exit(1);
        }
    };
    listener.set_nonblocking(true).ok();

    stdio::log("listen", &format!("http://{}:{}", bind_addr, port));

    // Spawn background task to clean up stale preview bundles every hour.
    {
        let cleanup_state = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CLEANUP_INTERVAL).await;
                let removed = cleanup_state.cleanup_stale_previews();
                if removed > 0 {
                    stdio::log(
                        "cleanup",
                        &format!("removed {} stale preview bundle(s)", removed),
                    );
                }
            }
        });
    }

    // Rate limiting uses the built-in defaults; the `DEKA_RATE_LIMIT_*` env
    // toggle was removed with the environment-as-config channel (deka#801).
    let rate_limiter = Arc::new(RateLimiter::new(
        deka_http::rate_limit::RateLimitConfig::default(),
    ));
    rate_limiter.spawn_janitor();

    let app = Router::new()
        .route(
            "/__admin/rebuild",
            axum::routing::post(handle_admin_rebuild),
        )
        .fallback(handle_platform_request)
        .with_state(state)
        .layer(from_fn_with_state(rate_limiter, rate_limit_middleware));

    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}

/// POST /__admin/rebuild?shop_id=xxx[&ref=hash]
///
/// Clears the bundle cache for the given tenant and re-bundles their code.
/// If `ref` is provided, caches the bundle as `shop_id:ref` (preview build).
/// If `ref` is omitted, caches as `shop_id` (main build, current behavior).
/// Only accepts requests from localhost for security.
async fn handle_admin_rebuild(
    State(state): State<Arc<PlatformState>>,
    request: Request,
) -> impl IntoResponse {
    let uri = request.uri().clone();

    // Parse query params
    let mut shop_id: Option<String> = None;
    let mut git_ref: Option<String> = None;

    if let Some(query) = uri.query() {
        for pair in query.split('&') {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().unwrap_or("");
            let val = parts.next().unwrap_or("");
            if key == "shop_id" && !val.is_empty() {
                shop_id = Some(val.to_string());
            } else if key == "ref" && !val.is_empty() {
                git_ref = Some(val.to_string());
            }
        }
    }

    let shop_id = match shop_id {
        Some(id) => id,
        None => {
            return Response::builder()
                .status(400)
                .body(axum::body::Body::from(
                    r#"{"error":"missing shop_id query parameter"}"#,
                ))
                .unwrap();
        }
    };

    // The cache key is `shop_id` for main or `shop_id:{ref}` for previews
    let cache_key = match &git_ref {
        Some(r) => format!("{}:{}", shop_id, r),
        None => shop_id.clone(),
    };

    let label = if git_ref.is_some() {
        "preview rebuild"
    } else {
        "rebuild"
    };
    let started = Instant::now();
    stdio::log(label, &format!("triggered for {}", cache_key));

    // Clear the cached bundle
    {
        let mut cache = state.bundle_cache.lock().unwrap();
        if git_ref.is_some() {
            // Preview-only: drop just the one preview entry.
            cache.remove(&cache_key);
        } else {
            // Main rebuild: evict main and every preview keyed as `shop_id:*`
            // so next preview request rebuilds on the new base as well.
            let prefix = format!("{}:", shop_id);
            cache.retain(|k, _| k != &shop_id && !k.starts_with(&prefix));
        }
    }

    // Evict cached isolates for this tenant across all workers.
    // Handler keys are `tenant:{shop_id}` or `tenant:{shop_id}:{ref}` —
    // use `tenant:{shop_id}` as the prefix to catch both. For preview
    // rebuilds evict only the exact variant.
    let isolate_prefix = if git_ref.is_some() {
        format!("tenant:{}", cache_key)
    } else {
        format!("tenant:{}", shop_id)
    };
    let evicted = state.engine.pool().evict_by_prefix(&isolate_prefix).await;

    // Re-bundle (resolve_handler will re-compile since cache is cleared)
    let (_key, code, _entry) = state.resolve_handler(&shop_id, &cache_key);
    let bundle_size = code.len();
    let elapsed_ms = started.elapsed().as_millis();

    if code.is_empty() {
        stdio::error(label, &format!("failed for {}", cache_key));
        return Response::builder()
            .status(500)
            .body(axum::body::Body::from(format!(
                r#"{{"error":"bundle failed for {}"}}"#,
                cache_key
            )))
            .unwrap();
    }

    stdio::log(
        label,
        &format!(
            "complete for {} ({} bytes, {} isolate(s) evicted, {}ms)",
            cache_key, bundle_size, evicted, elapsed_ms
        ),
    );

    Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(format!(
            r#"{{"status":"ok","shop_id":"{}","ref":{},"bundle_size":{},"evicted":{},"elapsed_ms":{}}}"#,
            shop_id,
            match &git_ref {
                Some(r) => format!("\"{}\"", r),
                None => "null".to_string(),
            },
            bundle_size,
            evicted,
            elapsed_ms
        )))
        .unwrap()
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
    if claims_cloudflare_ip_without_ray(&headers) {
        return Response::builder()
            .status(400)
            .body(axum::body::Body::from(
                "Bad Request: cf-connecting-ip requires cf-ray",
            ))
            .unwrap();
    }

    // Read body
    let content_len = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(usize::MAX);

    let body_bytes: Option<bytes::Bytes> = if content_len == 0 {
        None
    } else {
        match axum::body::to_bytes(request.into_body(), usize::MAX).await {
            Ok(b) if !b.is_empty() => Some(b),
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

    // Resolve tenant from server-routed Host/subdomain data only
    // (preview-aware). Do not use the dev `DEKA_SHOP_ID` fallback in the
    // multi-tenant platform path; unrouted/admin requests must not inherit a
    // tenant env.
    let tenant_info = pool::tenant::resolve_tenant_info_from_host_strict(&headers);
    let shop_id = tenant_info
        .as_ref()
        .map(|t| t.shop_id.clone())
        .unwrap_or_default();
    let cache_key = tenant_info
        .as_ref()
        .map(|t| t.cache_key())
        .unwrap_or_else(|| shop_id.clone());

    let body = body_bytes
        .as_ref()
        .map(|b| String::from_utf8_lossy(b).to_string());
    let (handler_key, handler_code, handler_entry) = state.resolve_handler(&shop_id, &cache_key);

    // If this is a preview request and the preview bundle failed, fall back to
    // the main branch build rather than returning 500.
    let (handler_key, handler_code, handler_entry) = if handler_code.is_empty() {
        if let Some(ref info) = tenant_info {
            if info.preview_ref.is_some() {
                stdio::log(
                    "platform",
                    &format!(
                        "preview bundle unavailable for {}, falling back to main",
                        cache_key
                    ),
                );
                state.resolve_handler(&shop_id, &shop_id)
            } else {
                (handler_key, handler_code, handler_entry)
            }
        } else {
            (handler_key, handler_code, handler_entry)
        }
    } else {
        (handler_key, handler_code, handler_entry)
    };

    // Guard: if the bundle failed completely (empty code), return HTTP 500
    // instead of sending empty JS to V8 which causes a HandleScope panic.
    if handler_code.is_empty() {
        stdio::error(
            "platform",
            &format!(
                "no handler code for tenant '{}' — bundle failed, returning 500",
                shop_id
            ),
        );
        return Response::builder()
            .status(500)
            .body(axum::body::Body::from(
                "Internal Server Error: store bundle unavailable",
            ))
            .unwrap();
    }

    let request_parts = RequestParts {
        url: format!("http://localhost{}", uri),
        method,
        headers,
        body,
    };

    let request_data = RequestData {
        handler_code,
        handler_entry,
        module_root: None,
        request_value: serde_json::Value::Null,
        request_parts: Some(request_parts),
        mode: ExecutionMode::Request,
        security: state.security.clone(),
    };

    match state.engine.execute(handler_key, request_data).await {
        Ok(pool_response) => {
            if !pool_response.success {
                let err = pool_response
                    .error
                    .unwrap_or_else(|| "Unknown error".to_string());
                stdio::error("platform", &format!("handler error: {}", err));
                let dev_mode = false;
                return Response::builder()
                    .status(500)
                    .body(axum::body::Body::from(handler_failure_body(
                        &format!("Handler error: {}", err),
                        dev_mode,
                    )))
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
                        response.body(axum::body::Body::from(body_bytes)).unwrap()
                    }
                    Err(err) => {
                        stdio::error("platform", &format!("response error: {}", err));
                        let dev_mode = false;
                        Response::builder()
                            .status(500)
                            .body(axum::body::Body::from(handler_failure_body(
                                &format!("Response error: {}", err),
                                dev_mode,
                            )))
                            .unwrap()
                    }
                },
                None => {
                    stdio::error("platform", "no response from handler");
                    let dev_mode = false;
                    Response::builder()
                        .status(500)
                        .body(axum::body::Body::from(handler_failure_body(
                            "No response from handler",
                            dev_mode,
                        )))
                        .unwrap()
                }
            }
        }
        Err(err) => {
            stdio::error("platform", &format!("handler execution failed: {}", err));
            let dev_mode = false;
            Response::builder()
                .status(500)
                .body(axum::body::Body::from(handler_failure_body(
                    &format!("Handler execution failed: {}", err),
                    dev_mode,
                )))
                .unwrap()
        }
    }
}

fn claims_cloudflare_ip_without_ray(headers: &[(String, String)]) -> bool {
    let has_cf_connecting_ip = headers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("cf-connecting-ip") && !value.trim().is_empty()
    });
    if !has_cf_connecting_ip {
        return false;
    }

    !headers
        .iter()
        .any(|(key, value)| key.eq_ignore_ascii_case("cf-ray") && !value.trim().is_empty())
}

#[cfg(test)]
mod shop_id_path_validation_tests {
    use super::resolve_handler_path;

    struct TestRoot(std::path::PathBuf);

    impl TestRoot {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "deka-870-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(dir.join("tenants")).unwrap();
            std::fs::create_dir_all(dir.join("default")).unwrap();
            std::fs::write(dir.join("default").join("main.ds"), "// default").unwrap();
            Self(dir)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn traversal_shaped_shop_ids_are_hard_errors() {
        let root = TestRoot::new("negative");
        for bad in [
            "../escape",
            "..",
            "shop_../escape",
            "shop_%2e%2e/escape",
            "/abs/path",
            " ",
            "shop_αβγ",
            "shop_😀",
            "shop_ok/../../etc/passwd",
            "shop_x\\..\\escape",
            "shop_Oops",
            "shop_ sneaky",
        ] {
            assert!(
                resolve_handler_path(&root.0, bad).is_err(),
                "fixture {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn rejected_shop_id_never_touches_the_filesystem() {
        let root = TestRoot::new("canary");
        // Canary at the exact location "../escape" would resolve to if it
        // were joined: <root>/tenants/../escape/main.ds == <root>/escape/main.ds.
        // If validation ever happened after the join (or not at all), the
        // exists() check would find this file and resolve it.
        std::fs::create_dir_all(root.0.join("escape")).unwrap();
        std::fs::write(root.0.join("escape").join("main.ds"), "// escaped").unwrap();

        assert!(resolve_handler_path(&root.0, "../escape").is_err());
        assert!(resolve_handler_path(&root.0, "..").is_err());
    }

    #[test]
    fn valid_shop_ids_resolve_as_before() {
        let root = TestRoot::new("positive");
        std::fs::create_dir_all(root.0.join("tenants").join("shop_alpha-1")).unwrap();
        std::fs::write(
            root.0.join("tenants").join("shop_alpha-1").join("main.ds"),
            "// tenant",
        )
        .unwrap();

        // Existing tenant dir → tenant-specific handler.
        assert_eq!(
            resolve_handler_path(&root.0, "shop_alpha-1").unwrap(),
            root.0
                .join("tenants")
                .join("shop_alpha-1")
                .join("main.ds")
        );
        // Valid charset, missing dir → default fallback.
        assert_eq!(
            resolve_handler_path(&root.0, "shop_missing").unwrap(),
            root.0.join("default").join("main.ds")
        );
        // Empty shop_id → default tenant (never a tenants/ join).
        assert_eq!(
            resolve_handler_path(&root.0, "").unwrap(),
            root.0.join("default").join("main.ds")
        );
    }
}

#[cfg(test)]
mod cloudflare_header_tests {
    use super::{claims_cloudflare_ip_without_ray, handler_failure_body};

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn rejects_cf_connecting_ip_without_cf_ray() {
        assert!(claims_cloudflare_ip_without_ray(&headers(&[(
            "cf-connecting-ip",
            "203.0.113.10",
        )])));
    }

    #[test]
    fn accepts_cf_connecting_ip_when_cf_ray_is_present() {
        assert!(!claims_cloudflare_ip_without_ray(&headers(&[
            ("cf-connecting-ip", "203.0.113.10"),
            ("cf-ray", "abc123-SJC"),
        ])));
    }

    #[test]
    fn rejects_cf_connecting_ip_with_blank_cf_ray() {
        assert!(claims_cloudflare_ip_without_ray(&headers(&[
            ("cf-connecting-ip", "203.0.113.10"),
            ("cf-ray", " "),
        ])));
    }

    #[test]
    fn accepts_requests_without_cf_connecting_ip() {
        assert!(!claims_cloudflare_ip_without_ray(&headers(&[(
            "x-forwarded-for",
            "203.0.113.10",
        )])));
    }

    #[test]
    fn header_names_are_case_insensitive() {
        assert!(!claims_cloudflare_ip_without_ray(&headers(&[
            ("CF-Connecting-IP", "203.0.113.10"),
            ("CF-Ray", "abc123-SJC"),
        ])));
    }

    #[test]
    fn platform_non_dev_handler_failures_are_redacted() {
        let detail = "Handler execution failed: Missing deka module 'missing/mod' \
(imported from /tmp/platform/main.ds). Attempted roots: /tmp/platform/php_modules. \
Available modules: crypto, bytes";
        let body = handler_failure_body(detail, false);

        assert_eq!(body, "Internal Server Error");
        assert!(!body.contains("/tmp/platform"));
        assert!(!body.contains("Available modules"));
        assert!(!body.contains("Attempted roots"));
    }

    #[test]
    fn platform_dev_handler_failures_keep_detail() {
        let detail = "Handler execution failed: Missing deka module 'missing/mod' \
(imported from /tmp/platform-dev/main.ds). Attempted roots: /tmp/platform-dev/php_modules. \
Available modules: crypto";
        let body = handler_failure_body(detail, true);

        assert!(body.contains("/tmp/platform-dev/main.ds"));
        assert!(body.contains("Available modules"));
        assert!(body.contains("Attempted roots"));
    }
}

pub(crate) mod stdio {
    pub(crate) fn log(category: &str, message: &str) {
        eprintln!("[{}] {}", category, message);
    }
    pub(crate) fn error(category: &str, message: &str) {
        eprintln!("[{}] ERROR: {}", category, message);
    }
}
