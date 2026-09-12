use std::sync::Arc;

use axum::extract::Extension;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::header::CONTENT_LENGTH;
use axum::middleware::from_fn_with_state;
use axum::{
    Router,
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use base64::Engine;

use crate::config::{HttpConfig, Neo4jConfig};
use crate::utility_css::{UtilityCssConfig, inject_utility_css};
use crate::websocket::{handle_hmr_websocket, handle_websocket, set_hmr_runtime_state};
use engine::{RuntimeState, execute_request_parts};

use crate::rate_limit::{RateLimiter, middleware as rate_limit_middleware};

/// Request-path configuration installed by the caller (deka#801). Replaces
/// the per-request `DEKA_*` env reads; built once in
/// `app_router_with_rate_limiter` and carried as an axum extension.
#[derive(Clone)]
struct HttpExtensions {
    debug: bool,
    platform_api: bool,
    neo4j: Arc<Neo4jConfig>,
    utility_css: UtilityCssConfig,
}

pub fn app_router(state: Arc<RuntimeState>, config: HttpConfig) -> Router {
    let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit.clone()));
    rate_limiter.spawn_janitor();
    app_router_with_rate_limiter(state, rate_limiter, config)
}

pub fn app_router_with_rate_limiter(
    state: Arc<RuntimeState>,
    rate_limiter: Arc<RateLimiter>,
    config: HttpConfig,
) -> Router {
    set_hmr_runtime_state(Arc::clone(&state));
    let extensions = HttpExtensions {
        debug: config.debug,
        platform_api: config.platform_api,
        neo4j: Arc::new(config.neo4j),
        utility_css: crate::utility_css::load_config(config.project_root.as_deref()),
    };
    Router::new()
        .fallback(handle_request)
        .with_state(state)
        .layer(Extension(extensions))
        .layer(from_fn_with_state(rate_limiter, rate_limit_middleware))
}

async fn handle_request(
    State(state): State<Arc<RuntimeState>>,
    Extension(extensions): Extension<HttpExtensions>,
    ws: Option<WebSocketUpgrade>,
    request: Request,
) -> impl IntoResponse {
    let method = request.method().as_str().to_string();
    let uri = request.uri().to_string();
    let path = request.uri().path().to_string();
    if let Some(response) = try_public_response(&state, &path) {
        return response;
    }
    if let Some(response) = try_asset_response(&state, &path) {
        return response;
    }

    // ── Built-in REST API (/api/*) — skip V8 isolate entirely ──
    // Only active when the caller enables it (HttpConfig::platform_api;
    // deka#801 — the `DEKA_PLATFORM_API` env toggle is gone). Standalone
    // `deka serve` apps own their own /api/* routes.
    if extensions.platform_api && (path.starts_with("/api/") || path == "/api") {
        let mut headers = Vec::with_capacity(request.headers().len());
        for (key, value) in request.headers().iter() {
            headers.push((
                key.as_str().to_string(),
                value.to_str().unwrap_or("").to_string(),
            ));
        }
        return crate::api::handle_api_request(&path, &headers, &extensions.neo4j)
            .await
            .into_response();
    }

    let hmr_path = path == "/_deka/hmr";
    if hmr_path && state.dev_mode {
        if let Some(ws) = ws {
            let state_for_hmr = Arc::clone(&state);
            return ws
                .on_upgrade(move |socket| handle_hmr_websocket(socket, state_for_hmr))
                .into_response();
        }
        return Response::builder()
            .status(426)
            .body(axum::body::Body::from("WebSocket upgrade required"))
            .unwrap();
    }
    if extensions.debug {
        tracing::info!("[http] request {} {}", method, uri);
    }
    let (headers, body) = if state.perf_mode {
        (Vec::new(), None)
    } else {
        let mut headers = Vec::with_capacity(request.headers().len());
        for (key, value) in request.headers().iter() {
            headers.push((
                key.as_str().to_string(),
                value.to_str().unwrap_or("").to_string(),
            ));
        }

        let content_len = request
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(usize::MAX);

        let body = if content_len == 0 {
            None
        } else {
            match axum::body::to_bytes(request.into_body(), usize::MAX).await {
                Ok(bytes) => {
                    if bytes.is_empty() {
                        None
                    } else {
                        Some(String::from_utf8_lossy(&bytes).to_string())
                    }
                }
                Err(_) => None,
            }
        };
        (headers, body)
    };

    match execute_request_parts(
        Arc::clone(&state),
        format!("http://localhost{}", uri),
        method,
        headers,
        body,
    )
    .await
    {
        Ok(mut response_envelope) => {
            if extensions.debug {
                tracing::info!("[http] response {} {}", response_envelope.status, uri);
            }
            if let Some(upgrade) = response_envelope.upgrade {
                if let Some(ws) = ws {
                    return ws
                        .on_upgrade(move |socket| handle_websocket(socket, state, Some(upgrade)))
                        .into_response();
                }
                return Response::builder()
                    .status(426)
                    .body(axum::body::Body::from("WebSocket upgrade required"))
                    .unwrap();
            }

            let mut response = Response::builder().status(response_envelope.status);
            let is_html = is_html_response(&response_envelope.headers);
            let inject_dev_hmr = state.dev_mode
                && is_html
                && response_envelope.body_base64.is_none()
                && !response_envelope.body.is_empty();

            for (key, value) in response_envelope.headers {
                if key.eq_ignore_ascii_case("set-cookie") && value.contains('\n') {
                    for part in value.split('\n').filter(|part| !part.is_empty()) {
                        response = response.header(&key, part);
                    }
                    continue;
                }
                response = response.header(&key, value);
            }

            if let Some(body_base64) = response_envelope.body_base64 {
                let bytes: Vec<u8> = match base64::engine::general_purpose::STANDARD
                    .decode(body_base64.as_bytes())
                {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        return Response::builder()
                            .status(500)
                            .body(axum::body::Body::from(format!(
                                "Failed to decode body: {}",
                                err
                            )))
                            .unwrap();
                    }
                };
                response.body(axum::body::Body::from(bytes)).unwrap()
            } else {
                if inject_dev_hmr {
                    response_envelope.body = inject_hmr_client(&response_envelope.body);
                }
                if is_html {
                    response_envelope.body =
                        inject_utility_css(&response_envelope.body, extensions.utility_css);
                }
                response
                    .body(axum::body::Body::from(response_envelope.body))
                    .unwrap()
            }
        }
        Err(err) => {
            tracing::error!("Handler execution failed: {}", err);
            Response::builder()
                .status(500)
                .body(axum::body::Body::from(handler_failure_body(
                    &err,
                    state.dev_mode,
                )))
                .unwrap()
        }
    }
}

fn handler_failure_body(detail: &str, dev_mode: bool) -> String {
    if dev_mode {
        format!("Handler execution failed: {}", detail)
    } else {
        "Internal Server Error".to_string()
    }
}

fn try_asset_response(state: &Arc<RuntimeState>, path: &str) -> Option<Response> {
    if !path.starts_with("/assets/") {
        return None;
    }
    let rel = path.trim_start_matches("/assets/");
    if rel.is_empty() || rel.contains('\0') {
        return None;
    }
    let rel_path = std::path::Path::new(rel);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let entry = state.handler_entry.as_ref()?;
    let cache_assets = std::path::Path::new(entry).parent()?.join("assets");
    let (bytes, ctype) = read_static_file(&cache_assets, rel_path)?;
    Response::builder()
        .status(200)
        .header("content-type", ctype)
        .body(axum::body::Body::from(bytes))
        .ok()
}

/// Serve app-router `public/` files before evaluating a route handler.
/// The root is set only for an app-router project by `runtime::serve`.
fn try_public_response(state: &Arc<RuntimeState>, path: &str) -> Option<Response> {
    let root = state.public_dir.as_ref()?;
    if let Some(manifest) = &state.artifact_manifest {
        return try_artifact_client_response(manifest, root, path);
    }
    let rel = path.strip_prefix('/')?;
    let (bytes, ctype) = read_static_file(root, std::path::Path::new(rel))?;
    Response::builder()
        .status(200)
        .header("content-type", ctype)
        .body(axum::body::Body::from(bytes))
        .ok()
}

/// Serve a built client's manifest-described bytes. Static routes are routed
/// to their emitted `index.html` before the V8 handler is considered; regular
/// files (public/ and generated assets) retain their direct paths. An
/// undeclared client file is never served: the build descriptor is the
/// authority, and `read_client_payload` verifies the digest on every read.
fn try_artifact_client_response(
    manifest: &runtime_core::framework::ArtifactManifestV2,
    root: &std::path::Path,
    path: &str,
) -> Option<Response> {
    let rel = path.strip_prefix('/')?;
    if rel.contains('\0')
        || std::path::Path::new(rel).is_absolute()
        || std::path::Path::new(rel)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }

    let mut candidates = Vec::new();
    if rel.is_empty() {
        if let Some(index) = &manifest.client.index {
            candidates.push(index.clone());
        }
    } else {
        candidates.push(format!("client/{rel}"));
        // `/about` is the request spelling for the prerendered
        // `client/about/index.html`; paths that already look like files do
        // not receive a directory-index fallback.
        if std::path::Path::new(rel).extension().is_none() {
            candidates.push(format!("client/{rel}/index.html"));
        }
    }
    for payload_path in candidates {
        let Ok(bytes) = manifest.read_client_payload(root.parent()?, &payload_path) else {
            continue;
        };
        let rel_path = payload_path
            .strip_prefix("client/")
            .unwrap_or(&payload_path);
        let ctype = content_type(std::path::Path::new(rel_path));
        return Response::builder()
            .status(200)
            .header("content-type", ctype)
            .body(axum::body::Body::from(bytes))
            .ok();
    }
    None
}

fn read_static_file(
    root: &std::path::Path,
    rel: &std::path::Path,
) -> Option<(Vec<u8>, &'static str)> {
    if rel.as_os_str().is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    let root = std::fs::canonicalize(root).ok()?;
    let file = std::fs::canonicalize(root.join(rel)).ok()?;
    if !file.is_file() || !file.starts_with(&root) {
        return None;
    }
    let bytes = std::fs::read(&file).ok()?;
    let ctype = content_type(&file);
    Some((bytes, ctype))
}

fn content_type(file: &std::path::Path) -> &'static str {
    match file.extension().and_then(|extension| extension.to_str()) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn is_html_response(headers: &std::collections::HashMap<String, String>) -> bool {
    for (key, value) in headers {
        if key.eq_ignore_ascii_case("content-type")
            && value.to_ascii_lowercase().contains("text/html")
        {
            return true;
        }
    }
    false
}

fn inject_hmr_client(html: &str) -> String {
    const MARKER: &str = "__deka_hmr_client";
    if html.contains(MARKER) {
        return html.to_string();
    }
    // The dev HMR client ships as JavaScript-only fragments (compile-time
    // include_str!, zero runtime cost), split by concern: the hydrate import
    // specifier, the focus/form preservation helpers, and the patch
    // dispatcher (island comment-marker walker). The specifier stays the
    // LOGICAL "ui/client" — the document's inline import map resolves it to
    // the current hashed chunk, so content-hash rotations stay invisible
    // here. Fragments replace the former single giant literal, on which
    // three unrelated PRs collided in one night; per-concern files keep
    // unrelated HMR changes from textually conflicting.
    const SCRIPT: &str = concat!(
        r#"<script id="__deka_hmr_client" type="module">"#,
        include_str!("hmr_client/hydrate.js"),
        include_str!("hmr_client/helpers.js"),
        include_str!("hmr_client/patch.js"),
        include_str!("hmr_client/socket.js"),
        "</script>"
    );
    if let Some(idx) = html.rfind("</body>") {
        let mut out = String::with_capacity(html.len() + SCRIPT.len());
        out.push_str(&html[..idx]);
        out.push_str(SCRIPT);
        out.push_str(&html[idx..]);
        return out;
    }
    let mut out = String::with_capacity(html.len() + SCRIPT.len());
    out.push_str(html);
    out.push_str(SCRIPT);
    out
}

#[cfg(test)]
mod tests {
    use super::{handler_failure_body, inject_hmr_client, read_static_file};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project_dir() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("deka_public_asset_{nonce}"));
        fs::create_dir_all(&path).expect("create temp project");
        path
    }

    #[test]
    fn injects_before_body_close() {
        let html = "<html><body><h1>x</h1></body></html>";
        let out = inject_hmr_client(html);
        assert!(out.contains("__deka_hmr_client"));
        assert!(out.find("__deka_hmr_client").unwrap() < out.find("</body>").unwrap());
    }

    #[test]
    fn avoids_duplicate_injection() {
        let html = "<html><body><script id=\"__deka_hmr_client\"></script></body></html>";
        let out = inject_hmr_client(html);
        assert_eq!(out.matches("__deka_hmr_client").count(), 1);
    }

    #[test]
    fn hmr_client_includes_state_preservation_hooks() {
        let html = "<html><body><div id=\"app\"><input id=\"x\" value=\"1\" /></div></body></html>";
        let out = inject_hmr_client(html);
        assert!(out.contains("selectionStart"));
        assert!(out.contains("window.scrollTo"));
        assert!(out.contains("querySelectorAll(\"input,textarea,select\")"));
        assert!(out.contains("data-deka-id"));
        assert!(out.contains("setSelectionRange"));
    }

    #[test]
    fn public_files_stay_within_the_public_root() {
        let project = temp_project_dir();
        let public = project.join("public");
        fs::create_dir_all(&public).expect("create public");
        fs::write(public.join("style.css"), "body { color: orange; }").expect("write css");
        fs::write(project.join("private.txt"), "private").expect("write private file");

        let (bytes, ctype) =
            read_static_file(&public, std::path::Path::new("style.css")).expect("read public css");
        assert_eq!(bytes, b"body { color: orange; }");
        assert_eq!(ctype, "text/css; charset=utf-8");
        assert!(read_static_file(&public, std::path::Path::new("../private.txt")).is_none());
        let _ = fs::remove_dir_all(project);
    }

    #[test]
    fn hmr_client_imports_hydrate_from_ui_client_module() {
        let html = "<html><body><div id=\"app\"></div></body></html>";
        let out = inject_hmr_client(html);
        // Module script with a module-level hydrate handle, per the
        // helpers-as-modules rule (RFD 44): hydrate travels with the code
        // as the ui/client module, not as a window.deka.ui global. The dev
        // client imports the LOGICAL specifier "ui/client", which the
        // document's inline import map resolves to the current hashed chunk —
        // the map is keyed by specifier precisely so filename layouts (and
        // content-hash rotations) stay invisible here. The resolved URL
        // matches the islands entries' "./ui/client.<hash>.js" import, so
        // the module instance (and island registry) is shared.
        assert!(out.contains("<script id=\"__deka_hmr_client\" type=\"module\">"));
        assert!(out.contains("import(\"ui/client\")"));
        assert!(
            !out.contains("/assets/ui/client"),
            "dev client must not hardcode an asset path that content hashing rotates"
        );
        assert!(out.contains("hmrHydrate(targetNode)"));
        assert!(!out.contains("window.deka"));
        // Never defined anywhere in the tree; the guarded call was dead code.
        assert!(!out.contains("__dekaMountDeclarativeShadows"));
        // The socket must connect without waiting on the network fetch: a
        // top-level await here would hold HMR hostage to client.js loading.
        // Early patches degrade to no re-hydration via the typeof guard.
        assert!(!out.contains("await "));
    }

    #[test]
    fn hmr_client_has_one_module_script_opening() {
        let html = "<html><body><div id=\"app\"></div></body></html>";
        let out = inject_hmr_client(html);
        assert_eq!(
            out.matches("<script").count(),
            1,
            "the injector owns the one module script wrapper; its JavaScript fragments must not add HTML"
        );
        assert_eq!(out.matches("</script>").count(), 1);
    }

    #[test]
    fn handler_failure_body_redacts_outside_dev() {
        let detail = "Missing phpx module 'missing/mod' (imported from /tmp/deka-leak/main.phpx). \
Attempted roots: /tmp/deka-leak/php_modules, /opt/deka/php_modules. \
Available modules: crypto, bytes. Lockfile: /tmp/deka-leak/deka.lock";
        let prod_body = handler_failure_body(detail, false);

        assert_eq!(prod_body, "Internal Server Error");
        assert!(!prod_body.contains("/tmp/deka-leak"));
        assert!(!prod_body.contains("Available modules"));
        assert!(!prod_body.contains("Attempted roots"));
        assert!(!prod_body.contains("deka.lock"));

        let dev_body = handler_failure_body(detail, true);

        assert!(dev_body.contains("/tmp/deka-leak/main.phpx"));
        assert!(dev_body.contains("Available modules"));
        assert!(dev_body.contains("Attempted roots"));
    }
}
