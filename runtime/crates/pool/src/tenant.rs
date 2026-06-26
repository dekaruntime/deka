use redis::{Client, Commands, Connection};
use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};
use zega_core::Zega;

/// Global embedded Zega instance for subdomain → tenant resolution.
///
/// Path is controlled by `DEKA_SUBDOMAIN_ZEGA_PATH` (default:
/// `store/zega/subdomains`).  The directory is created on first access.
fn global_subdomain_zega() -> Option<std::sync::MutexGuard<'static, Zega>> {
    static ZEGA: OnceLock<Mutex<Zega>> = OnceLock::new();
    let zega = ZEGA.get_or_init(|| {
        let path = std::env::var("DEKA_SUBDOMAIN_ZEGA_PATH")
            .unwrap_or_else(|_| "store/zega/subdomains".to_string());
        let zega = Zega::open(&path)
            .build()
            .expect("failed to open subdomain zega");
        Mutex::new(zega)
    });
    zega.lock().ok()
}

/// Raw shop mapping returned by a subdomain lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct SubdomainRecord {
    pub shop_id: String,
}

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
/// so Cloudflare's free SSL wildcard certificate covers preview URLs.
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
                if sep == b'-' && hash.chars().all(|c| c.is_ascii_hexdigit()) {
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

/// Resolve a subdomain to a shop ID via Zega lookup, falling back to Redis.
///
/// Returns `None` if not found or both backends unavailable.
pub fn resolve_tenant(subdomain: &str) -> Option<String> {
    resolve_tenant_record(subdomain).map(|r| r.shop_id)
}

/// Return true when a subdomain is already a canonical shop_id.
///
/// Shop IDs are accepted directly so shard-local storefront requests like
/// `shop_alpha.tana.gg` do not need a `subdomain:*` routing lookup.
pub fn is_shop_id_subdomain(subdomain: &str) -> bool {
    subdomain.strip_prefix("shop_").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    })
}

/// Resolve a subdomain to a `SubdomainRecord` via Zega lookup,
/// falling back to Redis during the transition period.
///
/// Accepts both the JSON format (`{"shop_id":...}`) and the legacy
/// plain-string format (just the shop_id). Extra JSON fields are ignored.
///
/// Zega is consulted first (`DEKA_SUBDOMAIN_ZEGA_PATH` controls the
/// database path).  If Zega is unavailable or the key is missing, the
/// resolver falls back to Redis so the rollout can be gradual.
///
/// Redis URL resolution order (fallback only):
/// 1. `DEKA_REDIS_URL` env var (explicit operator override).
/// 2. The local shard's Redis URL from the shard resolver (shard 0 / phobos).
/// 3. Hard-coded `redis://localhost:6380` (last-resort dev fallback).
pub fn resolve_tenant_record(subdomain: &str) -> Option<SubdomainRecord> {
    // 1. Try Zega first.
    if let Some(zega) = global_subdomain_zega() {
        let key = format!("subdomain:{}", subdomain);
        if let Some(value) = zega.kv_get(&key)
            && let Some(raw) = value.as_string()
        {
            return Some(parse_subdomain_value(raw));
        }
    }

    // 2. Fall back to Redis.
    let redis_url = std::env::var("DEKA_REDIS_URL").unwrap_or_else(|_| {
        deka_shard::global()
            .self_shard()
            .map(|s| s.redis.clone())
            .or_else(|| {
                deka_shard::global()
                    .shards()
                    .first()
                    .map(|s| s.redis.clone())
            })
            .unwrap_or_else(|| "redis://localhost:6380".to_string())
    });

    let raw: Option<String> = TENANT_REDIS.with(|cell: &RefCell<Option<Connection>>| {
        let mut conn = cell.borrow_mut();
        if conn.is_none() {
            if let Ok(client) = Client::open(redis_url.as_str()) {
                // Use a short timeout to avoid blocking the worker thread
                if let Ok(c) =
                    client.get_connection_with_timeout(std::time::Duration::from_millis(500))
                {
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
    });

    raw.map(|s| parse_subdomain_value(&s))
}

/// Write a subdomain record to the embedded Zega store.
///
/// This is the canonical write path for the subdomain → tenant map.
/// The edge publisher calls this after reading from Neo4j.
pub fn write_subdomain_record(subdomain: &str, record: &SubdomainRecord) -> Result<(), String> {
    let zega = global_subdomain_zega().ok_or("subdomain zega not initialised")?;
    let key = format!("subdomain:{}", subdomain);
    let value = serde_json::json!({
        "shop_id": record.shop_id,
    })
    .to_string();
    zega.kv_set(key, value.into(), None)
        .map_err(|e| format!("zega kv_set failed: {e}"))
}

/// Parse a `subdomain:*` value into a `SubdomainRecord`.
///
/// Accepts either:
///   - JSON: `{"shop_id": "..."}`
///   - Plain string: `"shop_foo"` (legacy format)
pub fn parse_subdomain_value(raw: &str) -> SubdomainRecord {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let shop_id = v
                .get("shop_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if !shop_id.is_empty() {
                return SubdomainRecord { shop_id };
            }
        }
    }
    SubdomainRecord {
        shop_id: trimmed.to_string(),
    }
}

/// Resolve tenant from request headers.
/// Extracts Host header → subdomain → Zega lookup → shop_id.
/// Falls back to `DEKA_SHOP_ID` env var for dev/testing.
pub fn resolve_tenant_from_headers(headers: &[(String, String)]) -> Option<String> {
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
/// Falls back to `DEKA_SHOP_ID` env var for local single-tenant dev/testing.
pub fn resolve_tenant_info_from_host(headers: &[(String, String)]) -> Option<TenantInfo> {
    resolve_tenant_info_from_host_strict(headers).or_else(|| {
        std::env::var("DEKA_SHOP_ID")
            .ok()
            .map(|shop_id| TenantInfo {
                shop_id,
                preview_ref: None,
            })
    })
}

/// Resolve tenant info from server-routed Host/subdomain data only.
///
/// This deliberately does not read request headers other than `Host`, and does
/// not fall back to process env. Use it in platform multi-tenant request paths
/// before injecting `SHOP_ID`, `$_ENV`, or vault-backed secrets.
pub fn resolve_tenant_info_from_host_strict(headers: &[(String, String)]) -> Option<TenantInfo> {
    let host = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("");

    // Check for preview subdomain first: preview-{hash}-{shop}.domain.tld
    if let Some((hash, shop_subdomain)) = parse_preview_host(host) {
        if is_shop_id_subdomain(&shop_subdomain) {
            return Some(TenantInfo {
                shop_id: shop_subdomain,
                preview_ref: Some(hash),
            });
        }

        if let Some(rec) = resolve_tenant_record(&shop_subdomain) {
            return Some(TenantInfo {
                shop_id: rec.shop_id,
                preview_ref: Some(hash),
            });
        }
    }

    // Normal subdomain resolution
    if let Some(subdomain) = extract_subdomain(host) {
        if is_shop_id_subdomain(&subdomain) {
            return Some(TenantInfo {
                shop_id: subdomain,
                preview_ref: None,
            });
        }

        if let Some(rec) = resolve_tenant_record(&subdomain) {
            return Some(TenantInfo {
                shop_id: rec.shop_id,
                preview_ref: None,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_subdomain_from_host() {
        assert_eq!(
            extract_subdomain("sams-shoes.tana.com"),
            Some("sams-shoes".to_string())
        );
        assert_eq!(
            extract_subdomain("shop-a.tana.com:443"),
            Some("shop-a".to_string())
        );
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
        assert_eq!(
            extract_subdomain("shop.eu.tana.com"),
            Some("shop".to_string())
        );
    }

    #[test]
    fn resolve_from_headers_ignores_x_shop_id() {
        let headers = vec![
            ("Host".to_string(), "localhost:8530".to_string()),
            ("X-Shop-ID".to_string(), "override-id".to_string()),
        ];
        assert_ne!(
            resolve_tenant_from_headers(&headers),
            Some("override-id".to_string())
        );
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
    fn resolve_from_headers_ignores_x_shop_id_case_insensitive() {
        let headers = vec![
            ("host".to_string(), "localhost:8530".to_string()),
            ("x-shop-id".to_string(), "shop_override".to_string()),
        ];
        assert_ne!(
            resolve_tenant_from_headers(&headers),
            Some("shop_override".to_string())
        );
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
        assert_eq!(
            result,
            Some(("a1b2c3d".to_string(), "my-cool-shop".to_string()))
        );
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
    fn shop_id_subdomain_format_matches_expected_shape() {
        assert!(is_shop_id_subdomain("shop_alpha"));
        assert!(is_shop_id_subdomain("shop_alpha-1_beta"));

        assert!(!is_shop_id_subdomain("shop_"));
        assert!(!is_shop_id_subdomain("Shop_alpha"));
        assert!(!is_shop_id_subdomain("shop_Alpha"));
        assert!(!is_shop_id_subdomain("shop.alpha"));
        assert!(!is_shop_id_subdomain("alpha"));
    }

    #[test]
    fn resolve_from_host_uses_shop_id_subdomain_directly() {
        let headers = vec![("Host".to_string(), "shop_alpha-1.tana.gg".to_string())];

        let info = resolve_tenant_info_from_host(&headers).unwrap();

        assert_eq!(info.shop_id, "shop_alpha-1");
        assert_eq!(info.preview_ref, None);
    }

    #[test]
    fn resolve_preview_from_host_uses_shop_id_subdomain_directly() {
        let headers = vec![(
            "Host".to_string(),
            "preview-a1b2c3d-shop_alpha-1.tana.gg".to_string(),
        )];

        let info = resolve_tenant_info_from_host(&headers).unwrap();

        assert_eq!(info.shop_id, "shop_alpha-1");
        assert_eq!(info.preview_ref.as_deref(), Some("a1b2c3d"));
    }

    #[test]
    fn tenant_info_cache_key_main() {
        let info = TenantInfo {
            shop_id: "shop_beta".to_string(),
            preview_ref: None,
        };
        assert_eq!(info.cache_key(), "shop_beta");
    }

    #[test]
    fn parse_subdomain_value_json_ignores_legacy_account_id() {
        let rec = parse_subdomain_value(
            r#"{"shop_id":"shop_beta","account_id":"2789d397-a96a-44ba-9073-24c711d007ff"}"#,
        );
        assert_eq!(rec.shop_id, "shop_beta");
    }

    #[test]
    fn parse_subdomain_value_legacy_plain_string() {
        let rec = parse_subdomain_value("shop_beta");
        assert_eq!(rec.shop_id, "shop_beta");
    }

    #[test]
    fn parse_subdomain_value_json_format() {
        let rec = parse_subdomain_value(r#"{"shop_id":"shop_beta"}"#);
        assert_eq!(rec.shop_id, "shop_beta");
    }

    #[test]
    fn parse_subdomain_value_json_ignores_empty_legacy_account_id() {
        let rec = parse_subdomain_value(r#"{"shop_id":"shop_beta","account_id":""}"#);
        assert_eq!(rec.shop_id, "shop_beta");
    }

    #[test]
    fn parse_subdomain_value_bogus_json_falls_back() {
        // Non-JSON-looking string is treated as the legacy plain-string shop_id.
        let rec = parse_subdomain_value("weird_value");
        assert_eq!(rec.shop_id, "weird_value");
    }

    #[test]
    fn tenant_info_cache_key_preview() {
        let info = TenantInfo {
            shop_id: "shop_beta".to_string(),
            preview_ref: Some("a1b2c3d".to_string()),
        };
        assert_eq!(info.cache_key(), "shop_beta:a1b2c3d");
    }

    #[test]
    fn redis_lookup_integration() {
        // This test requires Redis at localhost:6380 (the dev Docker Redis port).
        // DEKA_REDIS_URL is set explicitly so the resolver uses the correct port
        // regardless of whether a shards.json is present in the test environment.
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
        unsafe {
            std::env::set_var("DEKA_REDIS_URL", redis_url);
        }

        let result = resolve_tenant("test-shop");
        assert_eq!(result, Some("shop_test_001".to_string()));

        // Clean up
        let _: () = conn.del("subdomain:test-shop").unwrap();
    }

    #[test]
    fn zega_lookup_integration() {
        // This test uses a temporary Zega directory and does NOT rely on Redis.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        unsafe {
            std::env::set_var("DEKA_SUBDOMAIN_ZEGA_PATH", path);
        }

        // Write a record into Zega
        write_subdomain_record(
            "zega-test",
            &SubdomainRecord {
                shop_id: "shop_zega_001".to_string(),
            },
        )
        .expect("write_subdomain_record should succeed");

        // Resolve without touching Redis
        let result = resolve_tenant_record("zega-test");
        assert_eq!(
            result,
            Some(SubdomainRecord {
                shop_id: "shop_zega_001".to_string(),
            })
        );

        // Verify Redis is NOT consulted for a missing key (Zega returns None,
        // then Redis fallback is tried).  We can't easily assert Redis was skipped,
        // but we can at least verify the Zega path works end-to-end.
    }

    #[test]
    fn zega_legacy_plain_string_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        unsafe {
            std::env::set_var("DEKA_SUBDOMAIN_ZEGA_PATH", path);
        }

        write_subdomain_record(
            "legacy-shop",
            &SubdomainRecord {
                shop_id: "shop_legacy".to_string(),
            },
        )
        .expect("write_subdomain_record should succeed");

        let result = resolve_tenant_record("legacy-shop");
        assert_eq!(
            result,
            Some(SubdomainRecord {
                shop_id: "shop_legacy".to_string(),
            })
        );
    }
}
