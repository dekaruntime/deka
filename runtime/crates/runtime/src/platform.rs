//! `deka platform` — multi-tenant serve mode.
//!
//! Each tenant gets their own PHPX handler from `tenants/{shop_id}/main.phpx`.
//! Falls back to `default/main.phpx` if the tenant dir doesn't exist.
//! Uses the bundler (same as `deka serve`) to compile PHPX→JS with stdlib prelude.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::response::{IntoResponse, Response};
use axum::Router;
use core::Context;
use engine::config as runtime_config;
use engine::{RuntimeEngine, set_engine};
use pool::{ExecutionMode, HandlerKey, PoolConfig, RequestData, RequestParts};

use crate::js_pipeline::build_phpx_handler_bundle;

/// Header that tags requests we've already proxied once. If we see it
/// and we STILL don't own the shard, we refuse to re-proxy (prevents
/// loops if the shard config is skewed across servers).
const PROXY_LOOP_HEADER: &str = "X-Deka-Proxied";

pub fn platform(context: &Context) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
    rt.block_on(platform_async(context));
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
}

/// How long preview builds stay in cache without being accessed (7 days).
const PREVIEW_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How often the cleanup task runs (every hour).
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

impl PlatformState {
    /// Returns (HandlerKey, handler_code, handler_entry) for a tenant.
    /// Uses the bundler to compile PHPX→JS with stdlib prelude baked in.
    /// Caches the result so subsequent requests are fast.
    ///
    /// `cache_key` is `shop_id` for main builds, `shop_id:{hash}` for previews.
    fn resolve_handler(&self, shop_id: &str, cache_key: &str) -> (HandlerKey, String, Option<String>) {
        let display_key = if cache_key.is_empty() { "default" } else { cache_key };

        // Check cache first
        {
            let mut cache = self.bundle_cache.lock().unwrap();
            if let Some(entry) = cache.get_mut(display_key) {
                entry.last_accessed = Instant::now();
                return (
                    HandlerKey::new(if shop_id.is_empty() { "default".to_string() } else { format!("tenant:{}", display_key) }),
                    entry.code.clone(),
                    None,
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
                stdio::log("platform", &format!("bundled {} ({} bytes)", display_key, bundled.len()));
                bundled
            }
            Err(err) => {
                stdio::error("platform", &format!("bundle failed for {}: {}", display_key, err));
                let default_path = self.root.join("default").join("main.phpx");
                let default_str = default_path.to_string_lossy().to_string();
                if handler_path != default_path {
                    match build_phpx_handler_bundle(&default_str) {
                        Ok(bundled) => {
                            stdio::log("platform", &format!(
                                "fallback to default for {} ({} bytes)", display_key, bundled.len()
                            ));
                            bundled
                        }
                        Err(err2) => {
                            stdio::error("platform", &format!(
                                "default fallback also failed for {}: {}", display_key, err2
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
            cache.insert(display_key.to_string(), BundleEntry {
                code: code.clone(),
                last_accessed: Instant::now(),
            });
        }

        (
            HandlerKey::new(if shop_id.is_empty() { "default".to_string() } else { format!("tenant:{}", display_key) }),
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
                stdio::log("cleanup", &format!("expired preview bundle: {} (idle {:?})", key, age));
                false
            } else {
                true
            }
        });
        before - cache.len()
    }
}

async fn platform_async(context: &Context) {
    crate::env::init_env();

    let input = &context.handler.input;
    let root = PathBuf::from(if input.is_empty() { "." } else { input });
    let root = std::fs::canonicalize(&root).unwrap_or(root);

    // Load database config from platform-level deka.json
    runtime_config::load_database_config(&root);

    // Install the process-global shard resolver from env. Any failure
    // falls through to the single-shard localhost default so dev
    // environments stay functional even without a shards.json.
    match deka_shard::ShardResolver::from_env() {
        Ok(resolver) => {
            let shards = resolver.shard_count();
            let self_name = resolver
                .self_shard()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "<none>".to_string());
            stdio::log(
                "shard",
                &format!(
                    "resolver loaded: {} shard(s), self = {}",
                    shards, self_name
                ),
            );
            let _ = deka_shard::set_global(resolver);
        }
        Err(err) => {
            stdio::error("shard", &format!("failed to load resolver: {}", err));
        }
    }

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

    // Bind address. Defaults to 127.0.0.1 for single-machine dev; in
    // sharded deployments the platform MUST be reachable from the
    // router (phobos) so cross-shard proxy requests can land. The
    // presence of DEKA_SHARD_SELF is a reliable signal we're in a
    // cluster — override explicitly via DEKA_PLATFORM_BIND.
    let bind_addr = std::env::var("DEKA_PLATFORM_BIND").unwrap_or_else(|_| {
        if std::env::var("DEKA_SHARD_SELF").is_ok() {
            "0.0.0.0".to_string()
        } else {
            "127.0.0.1".to_string()
        }
    });
    let listener = match TcpListener::bind(format!("{}:{}", bind_addr, port)) {
        Ok(l) => l,
        Err(err) => {
            stdio::error("platform", &format!("failed to bind port {}: {}", port, err));
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
                    stdio::log("cleanup", &format!("removed {} stale preview bundle(s)", removed));
                }
            }
        });
    }

    let app = Router::new()
        .route("/__admin/rebuild", axum::routing::post(handle_admin_rebuild))
        .fallback(handle_platform_request)
        .with_state(state);

    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    axum::serve(listener, app).await.unwrap();
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

    let label = if git_ref.is_some() { "preview rebuild" } else { "rebuild" };
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

    let already_proxied = headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case(PROXY_LOOP_HEADER));

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

    // Resolve tenant from Host header only (preview-aware).
    let tenant_info = pool::tenant::resolve_tenant_info_from_host(&headers);
    let shop_id = tenant_info.as_ref().map(|t| t.shop_id.clone()).unwrap_or_default();
    let cache_key = tenant_info.as_ref().map(|t| t.cache_key()).unwrap_or_else(|| shop_id.clone());

    // Cross-shard routing: if we know the shop's account_id and a
    // different shard owns it, transparently proxy the request over
    // the Tailscale mesh. Requests without an account_id (legacy
    // Redis entries, admin paths, health checks) serve locally —
    // shard 0 is the de-facto owner of "uncharted" traffic.
    if let Some(info) = tenant_info.as_ref() {
        if let Some(account_id) = info.account_id.as_ref() {
            let resolver = deka_shard::global();
            if !resolver.owns(account_id) {
                if let Some(target) = resolver.resolve(account_id) {
                    if already_proxied {
                        // A previous server thought we owned this shard
                        // but we don't. Refuse to bounce it again so we
                        // don't loop forever.
                        stdio::error(
                            "proxy",
                            &format!(
                                "refusing to re-proxy for account={} shop={} (target {}): loop guard triggered",
                                account_id, shop_id, target.name
                            ),
                        );
                        return Response::builder()
                            .status(500)
                            .body(axum::body::Body::from(
                                "Internal Server Error: shard routing loop",
                            ))
                            .unwrap();
                    }
                    return proxy_to_shard(
                        &target.name,
                        &method,
                        &uri,
                        &headers,
                        body_bytes,
                        resolver.self_shard().map(|s| s.index),
                    )
                    .await;
                }
            }
        }
    }

    let body = body_bytes
        .as_ref()
        .map(|b| String::from_utf8_lossy(b).to_string());
    let (handler_key, handler_code, handler_entry) = state.resolve_handler(&shop_id, &cache_key);

    // If this is a preview request and the preview bundle failed, fall back to
    // the main branch build rather than returning 500.
    let (handler_key, handler_code, handler_entry) = if handler_code.is_empty() {
        if let Some(ref info) = tenant_info {
            if info.preview_ref.is_some() {
                stdio::log("platform", &format!(
                    "preview bundle unavailable for {}, falling back to main", cache_key
                ));
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
        stdio::error("platform", &format!(
            "no handler code for tenant '{}' — bundle failed, returning 500", shop_id
        ));
        return Response::builder()
            .status(500)
            .body(axum::body::Body::from("Internal Server Error: store bundle unavailable"))
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

fn claims_cloudflare_ip_without_ray(headers: &[(String, String)]) -> bool {
    let has_cf_connecting_ip = headers
        .iter()
        .any(|(key, value)| {
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
mod tests {
    use super::claims_cloudflare_ip_without_ray;

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
}

/// Reverse-proxy a request to the shard that owns it.
///
/// Uses reqwest for simplicity — the body is already buffered (the
/// platform reads it eagerly into memory upstream), so streaming is
/// not an immediate win. Targets the shard server on its internal
/// Tailscale DNS name at the platform port 8530.
///
/// Preserves: method, headers (minus Host), and body. Injects
/// `X-Deka-Proxied: {self_index}` so the target can detect loops.
async fn proxy_to_shard(
    target_host: &str,
    method: &str,
    uri: &str,
    headers: &[(String, String)],
    body: Option<bytes::Bytes>,
    self_index: Option<usize>,
) -> Response {
    // Extract just the path+query — uri from axum may be an absolute URL.
    let path_and_query = match uri.find("://") {
        Some(scheme_end) => {
            let rest = &uri[scheme_end + 3..];
            rest.find('/').map(|slash| &rest[slash..]).unwrap_or("/")
        }
        None => uri,
    };
    let target_url = format!("http://{}:8530{}", target_host, path_and_query);

    let original_host = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();

    stdio::log(
        "proxy",
        &format!(
            "{} {} → {} (host={})",
            method, path_and_query, target_url, original_host
        ),
    );

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        // Don't follow redirects — tenant handlers may legitimately
        // return 3xx, and we want to forward those verbatim.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            stdio::error("proxy", &format!("client build failed: {}", err));
            return Response::builder()
                .status(502)
                .body(axum::body::Body::from("Bad Gateway: proxy client build failed"))
                .unwrap();
        }
    };

    let method_parsed = match reqwest::Method::from_bytes(method.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return Response::builder()
                .status(400)
                .body(axum::body::Body::from("Bad Request: unknown HTTP method"))
                .unwrap();
        }
    };

    let mut req = client.request(method_parsed, &target_url);

    // Forward headers except hop-by-hop + Host (reqwest sets Host from URL).
    // We leave the original Host as X-Forwarded-Host so the target shard's
    // platform can resolve the correct tenant from it.
    for (k, v) in headers {
        let kl = k.to_ascii_lowercase();
        if matches!(
            kl.as_str(),
            "host"
                | "content-length"
                | "connection"
                | "transfer-encoding"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailers"
                | "upgrade"
        ) {
            continue;
        }
        req = req.header(k, v);
    }
    if !original_host.is_empty() {
        req = req.header("Host", original_host.clone());
        req = req.header("X-Forwarded-Host", original_host);
    }
    req = req.header(PROXY_LOOP_HEADER, self_index.unwrap_or(usize::MAX).to_string());

    if let Some(b) = body {
        req = req.body(b);
    }

    let upstream = match req.send().await {
        Ok(r) => r,
        Err(err) => {
            stdio::error(
                "proxy",
                &format!("upstream {} failed: {}", target_url, err),
            );
            return Response::builder()
                .status(502)
                .body(axum::body::Body::from(format!(
                    "Bad Gateway: upstream {} unreachable",
                    target_host
                )))
                .unwrap();
        }
    };

    let status = upstream.status();
    let mut builder = Response::builder().status(status.as_u16());
    for (k, v) in upstream.headers().iter() {
        let kl = k.as_str().to_ascii_lowercase();
        if matches!(
            kl.as_str(),
            "connection"
                | "transfer-encoding"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailers"
                | "upgrade"
        ) {
            continue;
        }
        builder = builder.header(k.as_str(), v.as_bytes());
    }

    let bytes = match upstream.bytes().await {
        Ok(b) => b,
        Err(err) => {
            stdio::error("proxy", &format!("body read failed: {}", err));
            return Response::builder()
                .status(502)
                .body(axum::body::Body::from("Bad Gateway: upstream body read failed"))
                .unwrap();
        }
    };

    builder
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(502)
                .body(axum::body::Body::from("Bad Gateway: response build failed"))
                .unwrap()
        })
}

mod stdio {
    pub fn log(category: &str, message: &str) {
        eprintln!("[{}] {}", category, message);
    }
    pub fn error(category: &str, message: &str) {
        eprintln!("[{}] ERROR: {}", category, message);
    }
}
