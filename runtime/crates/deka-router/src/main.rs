//! Router-only Deka platform binary.
//!
//! This binary runs on edge nodes that are not shards. It resolves the owning
//! shard for a storefront request and proxies the HTTP request to that shard
//! without initialising the V8 isolate pool.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::response::{IntoResponse, Response};
use deka_shard::{ShardConfig, ShardInfo, ShardResolver};
use redis::Commands;

const PROXY_LOOP_HEADER: &str = "X-Deka-Proxied";
const DEFAULT_SHARDS_REDIS_KEY: &str = "deka:shards";
const DEFAULT_LISTEN_PORT: u16 = 8531;
const DEFAULT_TARGET_PORT: u16 = 8530;

#[derive(Clone)]
struct RouterState {
    resolver: Arc<ShardResolver>,
    client: reqwest::Client,
    target_port: u16,
    trust_shop_header: bool,
}

#[tokio::main]
async fn main() {
    let resolver = match load_resolver() {
        Ok(resolver) => resolver,
        Err(err) => {
            log_error("router", &format!("failed to load shard resolver: {err}"));
            std::process::exit(1);
        }
    };

    let shard_count = resolver.shard_count();
    if shard_count == 0 {
        log_error("router", "shard config is empty");
        std::process::exit(1);
    }
    log("router", &format!("loaded {shard_count} shard(s)"));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build proxy client");

    let state = Arc::new(RouterState {
        resolver: Arc::new(resolver),
        client,
        target_port: env_u16("DEKA_ROUTER_TARGET_PORT").unwrap_or(DEFAULT_TARGET_PORT),
        trust_shop_header: env_flag_enabled("DEKA_ROUTER_TRUST_SHOP_HEADER"),
    });

    let port = parse_port();
    let bind_addr = std::env::var("DEKA_ROUTER_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let listener = match TcpListener::bind(format!("{bind_addr}:{port}")) {
        Ok(listener) => listener,
        Err(err) => {
            log_error(
                "router",
                &format!("failed to bind {bind_addr}:{port}: {err}"),
            );
            std::process::exit(1);
        }
    };
    listener.set_nonblocking(true).ok();
    log("listen", &format!("http://{bind_addr}:{port}"));

    let app = Router::new()
        .route("/healthz", axum::routing::get(healthz))
        .fallback(route_request)
        .with_state(state);

    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn healthz() -> impl IntoResponse {
    Response::builder()
        .status(200)
        .body(Body::from("ok"))
        .unwrap()
}

async fn route_request(
    State(state): State<Arc<RouterState>>,
    request: Request,
) -> impl IntoResponse {
    let method = request.method().as_str().to_string();
    let uri = request.uri().to_string();

    let mut headers = Vec::with_capacity(request.headers().len());
    for (key, value) in request.headers().iter() {
        headers.push((
            key.as_str().to_string(),
            value.to_str().unwrap_or("").to_string(),
        ));
    }

    if headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case(PROXY_LOOP_HEADER))
    {
        return response(508, "Loop Detected: request was already proxied");
    }

    if claims_cloudflare_ip_without_ray(&headers) {
        return response(400, "Bad Request: cf-connecting-ip requires cf-ray");
    }

    let shop_id = match resolve_shop_id(&headers, state.trust_shop_header) {
        Some(shop_id) => shop_id,
        None => return response(400, "Bad Request: unable to resolve shop_id"),
    };

    let target = match state.resolver.resolve(&shop_id) {
        Some(target) => target,
        None => return response(502, "Bad Gateway: no shard for shop_id"),
    };

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
            Ok(bytes) if !bytes.is_empty() => Some(bytes),
            _ => None,
        }
    };

    proxy_to_shard(&state, target, &method, &uri, &headers, body).await
}

fn load_resolver() -> Result<ShardResolver, String> {
    let config = match load_shard_config_from_redis() {
        Some(config) => config,
        None => load_shard_config_from_file()?,
    };
    Ok(ShardResolver::from_config(config, None))
}

fn load_shard_config_from_redis() -> Option<ShardConfig> {
    let redis_url = match std::env::var("REDIS_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => return None,
    };
    let key = std::env::var("DEKA_SHARDS_REDIS_KEY")
        .unwrap_or_else(|_| DEFAULT_SHARDS_REDIS_KEY.to_string());
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(err) => {
            log_error(
                "router",
                &format!("invalid REDIS_URL, using file fallback: {err}"),
            );
            return None;
        }
    };
    let mut conn = match client.get_connection_with_timeout(Duration::from_millis(500)) {
        Ok(conn) => conn,
        Err(err) => {
            log_error(
                "router",
                &format!("redis unavailable, using file fallback: {err}"),
            );
            return None;
        }
    };
    let raw: Option<String> = match conn.get(&key) {
        Ok(raw) => raw,
        Err(err) => {
            log_error(
                "router",
                &format!("redis GET {key} failed, using file fallback: {err}"),
            );
            return None;
        }
    };
    match raw {
        Some(raw) if !raw.trim().is_empty() => match serde_json::from_str::<ShardConfig>(&raw) {
            Ok(config) => {
                log(
                    "router",
                    &format!("loaded shard config from redis key {key}"),
                );
                Some(config)
            }
            Err(err) => {
                log_error(
                    "router",
                    &format!("redis shard config in {key} is invalid, using file fallback: {err}"),
                );
                None
            }
        },
        _ => None,
    }
}

fn load_shard_config_from_file() -> Result<ShardConfig, String> {
    let resolver = ShardResolver::from_env()?;
    log("router", "loaded shard config from file/env fallback");
    Ok(ShardConfig {
        shards: resolver.shards().to_vec(),
    })
}

fn resolve_shop_id(headers: &[(String, String)], trust_shop_header: bool) -> Option<String> {
    if trust_shop_header
        && let Some(shop_id) = header_value(headers, "x-deka-shop-id")
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        return Some(shop_id.to_string());
    }

    let host = header_value(headers, "host").unwrap_or("");
    preview_shop_subdomain(host).or_else(|| extract_subdomain(host))
}

async fn proxy_to_shard(
    state: &RouterState,
    target: &ShardInfo,
    method: &str,
    uri: &str,
    headers: &[(String, String)],
    body: Option<bytes::Bytes>,
) -> Response {
    let path_and_query = path_and_query(uri);
    let target_authority = target_authority(target, state.target_port);
    let target_url = format!("http://{target_authority}{path_and_query}");
    let original_host = header_value(headers, "host").unwrap_or("").to_string();

    log(
        "proxy",
        &format!("{method} {path_and_query} -> {target_url} (host={original_host})"),
    );

    let method_parsed = match reqwest::Method::from_bytes(method.as_bytes()) {
        Ok(method) => method,
        Err(_) => return response(400, "Bad Request: unknown HTTP method"),
    };
    let mut req = state.client.request(method_parsed, &target_url);

    for (key, value) in headers {
        let lower = key.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
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
        req = req.header(key, value);
    }

    if !original_host.is_empty() {
        req = req.header("Host", original_host.clone());
        req = req.header("X-Forwarded-Host", original_host);
    }
    req = req.header(PROXY_LOOP_HEADER, "router");

    if let Some(body) = body {
        req = req.body(body);
    }

    let upstream = match req.send().await {
        Ok(response) => response,
        Err(err) => {
            log_error("proxy", &format!("upstream {target_url} failed: {err}"));
            return response(502, "Bad Gateway: upstream shard unreachable");
        }
    };

    let status = upstream.status();
    let mut builder = Response::builder().status(status.as_u16());
    for (key, value) in upstream.headers().iter() {
        let lower = key.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
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
        builder = builder.header(key.as_str(), value.as_bytes());
    }

    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            log_error("proxy", &format!("upstream body read failed: {err}"));
            return response(502, "Bad Gateway: upstream body read failed");
        }
    };

    builder
        .body(Body::from(bytes))
        .unwrap_or_else(|_| response(502, "Bad Gateway: failed to build upstream response"))
}

fn target_authority(target: &ShardInfo, default_port: u16) -> String {
    if target.name.contains(':') {
        target.name.clone()
    } else {
        format!("{}:{default_port}", target.name)
    }
}

fn path_and_query(uri: &str) -> &str {
    match uri.find("://") {
        Some(scheme_end) => {
            let rest = &uri[scheme_end + 3..];
            rest.find('/').map(|slash| &rest[slash..]).unwrap_or("/")
        }
        None => uri,
    }
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn extract_subdomain(host: &str) -> Option<String> {
    let host = host.split(':').next().unwrap_or(host);
    if host == "localhost" || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return None;
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 3 {
        Some(parts[0].to_string())
    } else {
        None
    }
}

fn preview_shop_subdomain(host: &str) -> Option<String> {
    let first = extract_subdomain(host)?;
    let rest = first.strip_prefix("preview-")?;
    if rest.len() <= 8 {
        return None;
    }
    let hash = &rest[..7];
    if rest.as_bytes()[7] == b'-' && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        let shop = &rest[8..];
        if !shop.is_empty() {
            return Some(shop.to_string());
        }
    }
    None
}

fn claims_cloudflare_ip_without_ray(headers: &[(String, String)]) -> bool {
    header_value(headers, "cf-connecting-ip").is_some() && header_value(headers, "cf-ray").is_none()
}

fn response(status: u16, body: &str) -> Response {
    Response::builder()
        .status(status)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn parse_port() -> u16 {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--port" {
            if let Some(value) = args.next()
                && let Ok(port) = value.parse()
            {
                return port;
            }
        } else if let Some(value) = arg.strip_prefix("--port=")
            && let Ok(port) = value.parse()
        {
            return port;
        }
    }
    env_u16("PORT").unwrap_or(DEFAULT_LISTEN_PORT)
}

fn env_u16(key: &str) -> Option<u16> {
    std::env::var(key).ok()?.parse().ok()
}

fn env_flag_enabled(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn log(category: &str, message: &str) {
    eprintln!("[{category}] {message}");
}

fn log_error(category: &str, message: &str) {
    eprintln!("[{category}] ERROR: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use deka_shard::{fnv1a_64, shard_index};

    fn two_shard_resolver() -> ShardResolver {
        ShardResolver::from_config(
            ShardConfig {
                shards: vec![
                    ShardInfo {
                        index: 0,
                        name: "local".to_string(),
                        neo4j: "bolt://localhost:7687".to_string(),
                        redis: "redis://localhost:6379".to_string(),
                    },
                    ShardInfo {
                        index: 1,
                        name: "bugsy".to_string(),
                        neo4j: "bolt://bugsy:7687".to_string(),
                        redis: "redis://bugsy:6379".to_string(),
                    },
                ],
            },
            None,
        )
    }

    fn shop_id_for_shard(resolver: &ShardResolver, index: usize) -> String {
        (0..10_000)
            .map(|n| format!("shop_router_{n}"))
            .find(|shop_id| resolver.resolve(shop_id).is_some_and(|s| s.index == index))
            .expect("test resolver should produce a shop_id for requested shard")
    }

    #[test]
    fn extracts_preview_shop_subdomain() {
        assert_eq!(
            preview_shop_subdomain("preview-a1b2c3d-alpha.tana.gg"),
            Some("alpha".to_string())
        );
    }

    #[test]
    fn resolves_shop_id_from_host_subdomain_without_directory_lookup() {
        let headers = vec![("Host".to_string(), "shop_alpha.tana.gg".to_string())];

        assert_eq!(
            resolve_shop_id(&headers, false),
            Some("shop_alpha".to_string())
        );
    }

    #[test]
    fn host_subdomain_maps_to_fnv1a_shard() {
        let resolver = two_shard_resolver();
        let shop_id = shop_id_for_shard(&resolver, 1);
        let headers = vec![("Host".to_string(), format!("{shop_id}.tana.gg"))];

        let resolved_shop_id = resolve_shop_id(&headers, false).unwrap();
        let target = resolver.resolve(&resolved_shop_id).unwrap();

        assert_eq!(resolved_shop_id, shop_id);
        assert_eq!(target.index, 1);
        assert_eq!(target.index, shard_index(&shop_id, resolver.shard_count()));
        assert_eq!(
            target.index,
            (fnv1a_64(shop_id.as_bytes()) % resolver.shard_count() as u64) as usize
        );
    }

    #[test]
    fn target_authority_uses_embedded_port() {
        let shard = ShardInfo {
            index: 0,
            name: "localhost:8532".to_string(),
            neo4j: "bolt://localhost:7687".to_string(),
            redis: "redis://localhost:6379".to_string(),
        };
        assert_eq!(target_authority(&shard, 8530), "localhost:8532");
    }

    #[test]
    fn target_authority_adds_default_port() {
        let shard = ShardInfo {
            index: 0,
            name: "phobos".to_string(),
            neo4j: "bolt://phobos:7687".to_string(),
            redis: "redis://phobos:6379".to_string(),
        };
        assert_eq!(target_authority(&shard, 8530), "phobos:8530");
    }
}
