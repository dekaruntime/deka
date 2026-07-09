use std::time::Duration;

use axum::response::Response;

use super::{PROXY_LOOP_HEADER, stdio};

pub(super) fn shard_key_from_host(headers: &[(String, String)]) -> Option<String> {
    let host = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("");

    if let Some((_, shop_subdomain)) = pool::tenant::parse_preview_host(host) {
        return Some(shop_subdomain);
    }

    pool::tenant::extract_subdomain(host)
}

pub(super) fn proxy_target_for_shop_id<'a>(
    shop_id: Option<&str>,
    resolver: &'a deka_shard::ShardResolver,
    dev_mode: bool,
) -> Option<&'a deka_shard::ShardInfo> {
    if dev_mode {
        return None;
    }

    let shop_id = shop_id.filter(|shop_id| !shop_id.is_empty())?;

    if resolver.owns(shop_id) {
        None
    } else {
        resolver.resolve(shop_id)
    }
}

/// Reverse-proxy a request to the shard that owns it.
///
/// Uses reqwest for simplicity because the platform already buffers request
/// bodies in memory. Targets the shard server on its internal DNS name at the
/// platform port 8530.
pub(super) async fn proxy_to_shard(
    target_host: &str,
    method: &str,
    uri: &str,
    headers: &[(String, String)],
    body: Option<bytes::Bytes>,
    self_index: Option<usize>,
) -> Response {
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
            "{} {} -> {} (host={})",
            method, path_and_query, target_url, original_host
        ),
    );

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            stdio::error("proxy", &format!("client build failed: {}", err));
            return Response::builder()
                .status(502)
                .body(axum::body::Body::from(
                    "Bad Gateway: proxy client build failed",
                ))
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
    req = req.header(
        PROXY_LOOP_HEADER,
        self_index.unwrap_or(usize::MAX).to_string(),
    );

    if let Some(b) = body {
        req = req.body(b);
    }

    let upstream = match req.send().await {
        Ok(r) => r,
        Err(err) => {
            stdio::error("proxy", &format!("upstream {} failed: {}", target_url, err));
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
                .body(axum::body::Body::from(
                    "Bad Gateway: upstream body read failed",
                ))
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

#[cfg(test)]
mod tests {
    use super::{proxy_target_for_shop_id, shard_key_from_host};
    use deka_shard::{ShardConfig, ShardInfo, ShardResolver, fnv1a_64, shard_index};

    fn two_shard_resolver(self_name: Option<&str>) -> ShardResolver {
        ShardResolver::from_config(
            ShardConfig {
                shards: vec![
                    ShardInfo {
                        index: 0,
                        name: "local".into(),
                        neo4j: "bolt://127.0.0.1:7688".into(),
                        redis: "redis://127.0.0.1:6380".into(),
                    },
                    ShardInfo {
                        index: 1,
                        name: "bugsy".into(),
                        neo4j: "bolt://bugsy:7687".into(),
                        redis: "redis://bugsy:6379".into(),
                    },
                ],
            },
            self_name,
        )
    }

    fn shop_id_for_shard(resolver: &ShardResolver, index: usize) -> String {
        (0..10_000)
            .map(|n| format!("shop_dev_created_{n}"))
            .find(|shop_id| resolver.resolve(shop_id).is_some_and(|s| s.index == index))
            .expect("test resolver should produce a shop_id for requested shard")
    }

    #[test]
    fn proxy_target_routes_remote_owner_in_production() {
        let resolver = two_shard_resolver(Some("local"));
        let shop_id = shop_id_for_shard(&resolver, 1);

        let target = proxy_target_for_shop_id(Some(&shop_id), &resolver, false).unwrap();

        assert_eq!(target.name, "bugsy");
    }

    #[test]
    fn host_subdomain_maps_to_expected_shard() {
        let resolver = two_shard_resolver(Some("local"));
        let shop_id = shop_id_for_shard(&resolver, 1);
        let headers = vec![("Host".to_string(), format!("{shop_id}.tana.gg"))];

        let host_shop_id = shard_key_from_host(&headers).unwrap();
        let target = proxy_target_for_shop_id(Some(&host_shop_id), &resolver, false).unwrap();

        assert_eq!(host_shop_id, shop_id);
        assert_eq!(target.index, 1);
        assert_eq!(target.index, shard_index(&shop_id, resolver.shard_count()));
        assert_eq!(
            target.index,
            (fnv1a_64(shop_id.as_bytes()) % resolver.shard_count() as u64) as usize
        );
    }

    #[test]
    fn preview_host_subdomain_maps_by_shop_slug_not_preview_slug() {
        let resolver = two_shard_resolver(Some("local"));
        let shop_id = shop_id_for_shard(&resolver, 1);
        let headers = vec![(
            "Host".to_string(),
            format!("preview-a1b2c3d-{shop_id}.tana.gg"),
        )];

        let host_shop_id = shard_key_from_host(&headers).unwrap();
        let target = proxy_target_for_shop_id(Some(&host_shop_id), &resolver, false).unwrap();

        assert_eq!(host_shop_id, shop_id);
        assert_eq!(target.index, shard_index(&shop_id, resolver.shard_count()));
    }

    #[test]
    fn proxy_target_serves_locally_in_dev_even_for_remote_owner() {
        let resolver = two_shard_resolver(Some("local"));
        let shop_id = shop_id_for_shard(&resolver, 1);

        assert!(proxy_target_for_shop_id(Some(&shop_id), &resolver, true).is_none());
    }

    #[test]
    fn proxy_target_serves_locally_for_owned_or_unrouted_hosts() {
        let resolver = two_shard_resolver(Some("local"));
        let local_shop_id = shop_id_for_shard(&resolver, 0);

        assert!(proxy_target_for_shop_id(Some(&local_shop_id), &resolver, false).is_none());
        assert!(proxy_target_for_shop_id(None, &resolver, false).is_none());
    }
}
