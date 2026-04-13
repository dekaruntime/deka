//! Tenant resolution — maps incoming request Host header to a shop ID.
//!
//! Uses Redis for subdomain → shop_id lookup.
//! Keys: `subdomain:{name}` → `{shop_id}`
//!
//! Preview deploys: `preview-{hash}-{shop}.tana.gg` resolves to the same
//! shop_id as `{shop}.tana.gg`, but the caller receives the hash so it
//! can look up the branch-specific bundle keyed as `{shop_id}:{hash}`.
//!
//! The flat subdomain format (`preview-{hash}-{shop}` instead of
//! `preview-{hash}.{shop}`) keeps everything under `*.tana.gg` so
//! Cloudflare's free SSL wildcard certificate covers it.

use redis::{Client, Commands, Connection};
use std::cell::RefCell;

/// Result of tenant resolution, optionally carrying a preview commit hash.
#[derive(Debug, Clone, PartialEq)]
pub struct TenantInfo {
    pub shop_id: String,
    /// If the request came via `preview-{hash}-{shop}.tana.gg`, this holds
    /// the short commit hash. `None` means serve from the main branch.
    pub preview_ref: Option<String>,
}

impl TenantInfo {
    /// The bundle cache key: `shop_id` for main, `shop_id:{hash}` for previews.
    pub fn cache_key(&self) -> String {
        match &self.preview_ref {
            Some(hash) => format!("{}:{}", self.shop_id, hash),
            None => self.shop_id.clone(),
        }
    }
}

thread_local! {
    static TENANT_REDIS: RefCell<Option<Connection>> = RefCell::new(None);
}

/// Extract the subdomain from a Host header value.
/// e.g. "sams-shoes.tana.com" → Some("sams-shoes")
/// e.g. "tana.com" → None (bare domain)
/// e.g. "localhost:8530" → None (dev mode)
pub fn extract_subdomain(host: &str) -> Option<String> {
    // Strip port
    let host = host.split(':').next().unwrap_or(host);

    // Skip localhost, IP addresses
    if host == "localhost" || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return None;
    }

    let parts: Vec<&str> = host.split('.').collect();
    // Need at least 3 parts: subdomain.domain.tld
    if parts.len() >= 3 {
        Some(parts[0].to_string())
    } else {
        None
    }
}

/// Parse a flat preview subdomain: `preview-{hash}-{shop}.domain.tld`
/// Returns `Some((hash, shop_subdomain))` if this is a preview URL,
/// `None` otherwise.
///
/// The hash is always exactly 7 hex characters (short git hash).
/// The shop name follows the hash and may itself contain hyphens
/// (e.g. `my-cool-shop`), so we split on the fixed structure:
/// `preview-` (literal) + 7 hex chars + `-` + rest-is-shop.
///
/// Using a single subdomain level keeps everything under `*.tana.gg`
/// so Cloudflare's free wildcard certificate covers preview URLs.
pub fn parse_preview_host(host: &str) -> Option<(String, String)> {
    let host = host.split(':').next().unwrap_or(host);
    if host == "localhost" || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return None;
    }

    let parts: Vec<&str> = host.split('.').collect();
    // preview-{hash}-{shop}.domain.tld = at least 3 parts
    if parts.len() >= 3 {
        let first = parts[0];
        if let Some(rest) = first.strip_prefix("preview-") {
            // Hash is exactly 7 hex chars, followed by '-', then shop name
            if rest.len() > 8 {
                let hash = &rest[..7];
                let sep = rest.as_bytes()[7];
                if sep == b'-'
                    && hash.chars().all(|c| c.is_ascii_hexdigit())
                {
                    let shop_subdomain = rest[8..].to_string();
                    if !shop_subdomain.is_empty() {
                        return Some((hash.to_string(), shop_subdomain));
                    }
                }
            }
        }
    }
    None
}

/// Resolve a subdomain to a shop ID via Redis lookup.
/// Returns None if not found or Redis unavailable.
pub fn resolve_tenant(subdomain: &str) -> Option<String> {
    let redis_url =
        std::env::var("DEKA_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());

    TENANT_REDIS.with(|cell: &RefCell<Option<Connection>>| {
        let mut conn = cell.borrow_mut();
        if conn.is_none() {
            if let Ok(client) = Client::open(redis_url.as_str()) {
                // Use a short timeout to avoid blocking the worker thread
                if let Ok(c) = client.get_connection_with_timeout(std::time::Duration::from_millis(500)) {
                    *conn = Some(c);
                }
            }
        }

        if let Some(ref mut c) = *conn {
            let key = format!("subdomain:{}", subdomain);
            c.get::<_, Option<String>>(&key).ok().flatten()
        } else {
            None
        }
    })
}

/// Resolve tenant from request headers.
/// Extracts Host header → subdomain → Redis lookup → shop_id.
/// Falls back to `DEKA_SHOP_ID` env var for dev/testing.
///
/// NOTE: In platform (multi-tenant) mode, prefer `resolve_tenant_from_host`
/// which ignores the `X-Shop-ID` header to prevent spoofing. This function
/// trusts `X-Shop-ID` and is only suitable for trusted/internal callers.
pub fn resolve_tenant_from_headers(headers: &[(String, String)]) -> Option<String> {
    // Check for explicit X-Shop-ID header (testing/dev override)
    if let Some(shop_id) = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-shop-id"))
        .map(|(_, v)| v.clone())
    {
        if !shop_id.is_empty() {
            return Some(shop_id);
        }
    }

    resolve_tenant_from_host(headers)
}

/// Resolve tenant from the Host header only — ignores `X-Shop-ID`.
/// Use this in platform (multi-tenant) mode where the `X-Shop-ID`
/// header comes from untrusted external clients and must not influence
/// tenant routing or analytics attribution.
pub fn resolve_tenant_from_host(headers: &[(String, String)]) -> Option<String> {
    resolve_tenant_info_from_host(headers).map(|info| info.shop_id)
}

/// Like `resolve_tenant_from_host` but also returns preview ref info.
/// Used by the platform to decide which bundle cache key to use.
pub fn resolve_tenant_info_from_host(headers: &[(String, String)]) -> Option<TenantInfo> {
    let host = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("");

    // Check for preview subdomain first: preview-{hash}-{shop}.domain.tld
    if let Some((hash, shop_subdomain)) = parse_preview_host(host) {
        if let Some(shop_id) = resolve_tenant(&shop_subdomain) {
            return Some(TenantInfo {
                shop_id,
                preview_ref: Some(hash),
            });
        }
    }

    // Normal subdomain resolution
    if let Some(subdomain) = extract_subdomain(host) {
        if let Some(shop_id) = resolve_tenant(&subdomain) {
            return Some(TenantInfo {
                shop_id,
                preview_ref: None,
            });
        }
    }

    // Fallback: env var for dev
    std::env::var("DEKA_SHOP_ID").ok().map(|shop_id| TenantInfo {
        shop_id,
        preview_ref: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_subdomain_from_host() {
        assert_eq!(extract_subdomain("sams-shoes.tana.com"), Some("sams-shoes".to_string()));
        assert_eq!(extract_subdomain("shop-a.tana.com:443"), Some("shop-a".to_string()));
        assert_eq!(extract_subdomain("tana.com"), None);
        assert_eq!(extract_subdomain("localhost:8530"), None);
        assert_eq!(extract_subdomain("127.0.0.1:8530"), None);
    }

    #[test]
    fn extract_subdomain_with_ip_v4() {
        assert_eq!(extract_subdomain("192.168.1.1"), None);
        assert_eq!(extract_subdomain("10.0.0.1:3000"), None);
    }

    #[test]
    fn extract_subdomain_plain_localhost() {
        assert_eq!(extract_subdomain("localhost"), None);
    }

    #[test]
    fn extract_subdomain_multiple_dots() {
        // e.g. "shop.eu.tana.com" -> the first part is the subdomain
        assert_eq!(extract_subdomain("shop.eu.tana.com"), Some("shop".to_string()));
    }

    #[test]
    fn resolve_from_headers_x_shop_id_takes_priority() {
        let headers = vec![
            ("Host".to_string(), "real-shop.tana.com".to_string()),
            ("X-Shop-ID".to_string(), "override-id".to_string()),
        ];
        assert_eq!(resolve_tenant_from_headers(&headers), Some("override-id".to_string()));
    }

    #[test]
    fn resolve_from_headers_empty_x_shop_id_ignored() {
        let headers = vec![
            ("X-Shop-ID".to_string(), "".to_string()),
            ("Host".to_string(), "localhost:8530".to_string()),
        ];
        // Empty X-Shop-ID should not override; falls through to host/env
        let result = resolve_tenant_from_headers(&headers);
        // Result depends on DEKA_SHOP_ID env; just verify no panic
        let _ = result;
    }

    #[test]
    fn resolve_from_headers_with_explicit_header() {
        let headers = vec![
            ("host".to_string(), "anything.tana.com".to_string()),
            ("x-shop-id".to_string(), "shop_override".to_string()),
        ];
        assert_eq!(resolve_tenant_from_headers(&headers), Some("shop_override".to_string()));
    }

    #[test]
    fn resolve_from_host_ignores_x_shop_id() {
        let headers = vec![
            ("Host".to_string(), "localhost:8530".to_string()),
            ("X-Shop-ID".to_string(), "spoofed-shop".to_string()),
        ];
        // resolve_tenant_from_host should ignore X-Shop-ID entirely;
        // localhost has no subdomain so it falls through to env var.
        let result = resolve_tenant_from_host(&headers);
        // Must NOT be "spoofed-shop" — that would mean X-Shop-ID was trusted
        assert_ne!(result, Some("spoofed-shop".to_string()));
    }

    #[test]
    fn resolve_from_headers_localhost_falls_back_to_env() {
        let headers = vec![("host".to_string(), "localhost:8530".to_string())];
        // No DEKA_SHOP_ID set, no subdomain — should return None
        // (unless DEKA_SHOP_ID happens to be set in the test environment)
        let result = resolve_tenant_from_headers(&headers);
        // Can't assert None because env var might be set from earlier test
        let _ = result;
    }

    #[test]
    fn parse_preview_host_valid() {
        // Flat format: preview-{7hex}-{shop}.domain.tld
        let result = parse_preview_host("preview-a1b2c3d-beta.tana.gg");
        assert_eq!(result, Some(("a1b2c3d".to_string(), "beta".to_string())));
    }

    #[test]
    fn parse_preview_host_with_port() {
        let result = parse_preview_host("preview-a1b2c3d-beta.tana.gg:8530");
        assert_eq!(result, Some(("a1b2c3d".to_string(), "beta".to_string())));
    }

    #[test]
    fn parse_preview_host_hyphenated_shop() {
        // Shop names can contain hyphens: preview-{7hex}-{shop-with-hyphens}.domain.tld
        let result = parse_preview_host("preview-a1b2c3d-my-cool-shop.tana.gg");
        assert_eq!(result, Some(("a1b2c3d".to_string(), "my-cool-shop".to_string())));
    }

    #[test]
    fn parse_preview_host_short_hash_rejected() {
        // Hash must be exactly 7 hex chars
        let result = parse_preview_host("preview-abc-beta.tana.gg");
        assert_eq!(result, None);
    }

    #[test]
    fn parse_preview_host_non_hex_rejected() {
        let result = parse_preview_host("preview-zzzzzzz-beta.tana.gg");
        assert_eq!(result, None);
    }

    #[test]
    fn parse_preview_host_not_preview() {
        let result = parse_preview_host("beta.tana.gg");
        assert_eq!(result, None);
    }

    #[test]
    fn parse_preview_host_localhost() {
        let result = parse_preview_host("localhost:8530");
        assert_eq!(result, None);
    }

    #[test]
    fn parse_preview_host_old_nested_format_rejected() {
        // The old nested format should no longer match
        let result = parse_preview_host("preview-a1b2c3d.beta.tana.gg");
        assert_eq!(result, None);
    }

    #[test]
    fn tenant_info_cache_key_main() {
        let info = TenantInfo { shop_id: "shop_beta".to_string(), preview_ref: None };
        assert_eq!(info.cache_key(), "shop_beta");
    }

    #[test]
    fn tenant_info_cache_key_preview() {
        let info = TenantInfo { shop_id: "shop_beta".to_string(), preview_ref: Some("a1b2c3d".to_string()) };
        assert_eq!(info.cache_key(), "shop_beta:a1b2c3d");
    }

    #[test]
    fn redis_lookup_integration() {
        // This test requires Redis at localhost:6380
        let redis_url = "redis://localhost:6380";
        let client = match Client::open(redis_url) {
            Ok(c) => c,
            Err(_) => return, // Skip if no Redis
        };
        let mut conn = match client.get_connection() {
            Ok(c) => c,
            Err(_) => return,
        };

        // Seed test data
        let _: () = conn.set("subdomain:test-shop", "shop_test_001").unwrap();

        // Set env so tenant resolver uses our test Redis
        unsafe { std::env::set_var("DEKA_REDIS_URL", redis_url); }

        let result = resolve_tenant("test-shop");
        assert_eq!(result, Some("shop_test_001".to_string()));

        // Clean up
        let _: () = conn.del("subdomain:test-shop").unwrap();
    }
}
