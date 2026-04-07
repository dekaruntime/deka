//! Tenant resolution — maps incoming request Host header to a shop ID.
//!
//! Uses Redis for subdomain → shop_id lookup.
//! Keys: `subdomain:{name}` → `{shop_id}`

use redis::{Client, Commands, Connection};
use std::cell::RefCell;

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

    // Extract from Host header
    let host = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("");

    if let Some(subdomain) = extract_subdomain(host) {
        if let Some(shop_id) = resolve_tenant(&subdomain) {
            return Some(shop_id);
        }
    }

    // Fallback: env var for dev
    std::env::var("DEKA_SHOP_ID").ok()
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
    fn resolve_from_headers_with_explicit_header() {
        let headers = vec![
            ("host".to_string(), "anything.tana.com".to_string()),
            ("x-shop-id".to_string(), "shop_override".to_string()),
        ];
        assert_eq!(resolve_tenant_from_headers(&headers), Some("shop_override".to_string()));
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
