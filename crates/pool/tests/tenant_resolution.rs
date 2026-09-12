/// Issue #10 — Tenant handler resolution tests.
///
/// Tests the tenant resolution logic: subdomain extraction, untrusted header
/// handling, and the handler path resolution pattern used by the platform.
///
/// Cloudflare edge routing presents a canonical `shop_*` subdomain; the
/// resolver accepts that live path without an external shard lookup.
use pool::tenant::{extract_subdomain, resolve_tenant_from_headers};
use std::path::PathBuf;

// ── extract_subdomain ───────────────────────────────────────────────────

#[test]
fn subdomain_extracted_from_full_host() {
    assert_eq!(
        extract_subdomain("sams-shoes.tana.gg"),
        Some("sams-shoes".to_string())
    );
}

#[test]
fn subdomain_extracted_with_port() {
    assert_eq!(
        extract_subdomain("mystore.tana.local:8530"),
        Some("mystore".to_string())
    );
}

#[test]
fn bare_domain_returns_none() {
    assert_eq!(extract_subdomain("tana.gg"), None);
}

#[test]
fn localhost_returns_none() {
    assert_eq!(extract_subdomain("localhost"), None);
    assert_eq!(extract_subdomain("localhost:8530"), None);
}

#[test]
fn ip_address_returns_none() {
    assert_eq!(extract_subdomain("127.0.0.1"), None);
    assert_eq!(extract_subdomain("192.168.1.100:8080"), None);
}

// ── resolve_tenant_from_headers (untrusted header handling) ────────────

#[test]
fn x_shop_id_header_is_ignored() {
    let headers = vec![
        ("Host".to_string(), "localhost:8530".to_string()),
        ("X-Shop-ID".to_string(), "shop_alpha".to_string()),
    ];
    assert_ne!(
        resolve_tenant_from_headers(&headers),
        Some("shop_alpha".to_string())
    );
}

#[test]
fn x_shop_id_case_insensitive_is_ignored() {
    let headers = vec![("x-shop-id".to_string(), "shop_beta".to_string())];
    assert_ne!(
        resolve_tenant_from_headers(&headers),
        Some("shop_beta".to_string())
    );
}

#[test]
fn empty_x_shop_id_is_ignored() {
    // Empty X-Shop-ID falls through to Host-based resolution.
    let headers = vec![
        ("X-Shop-ID".to_string(), String::new()),
        ("Host".to_string(), "localhost:8530".to_string()),
    ];
    assert_eq!(resolve_tenant_from_headers(&headers), None);
}

#[test]
fn canonical_shop_subdomain_resolves_without_external_lookup() {
    let headers = vec![("Host".to_string(), "shop_alpha.tana.gg".to_string())];
    assert_eq!(
        resolve_tenant_from_headers(&headers),
        Some("shop_alpha".to_string())
    );
}

// ── Handler path resolution pattern (mirrors platform.rs logic) ─────────

/// Replicate the handler path resolution logic from PlatformState::resolve_handler
/// to test it in isolation without needing the full platform runtime.
fn resolve_handler_path(root: &std::path::Path, shop_id: &str) -> PathBuf {
    if shop_id.is_empty() {
        root.join("default").join("main.phpx")
    } else {
        let tenant = root.join("tenants").join(shop_id).join("main.phpx");
        if tenant.exists() {
            tenant
        } else {
            root.join("default").join("main.phpx")
        }
    }
}

#[test]
fn tenant_resolution_finds_handler() {
    // Set up a temp tenant directory with a main.phpx
    let tmp = std::env::temp_dir().join(format!(
        "deka_tenant_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let tenant_dir = tmp.join("tenants").join("shop_alpha");
    let default_dir = tmp.join("default");
    std::fs::create_dir_all(&tenant_dir).expect("create tenant dir");
    std::fs::create_dir_all(&default_dir).expect("create default dir");
    std::fs::write(
        tenant_dir.join("main.phpx"),
        "function App($req: mixed) { return 'tenant' }",
    )
    .expect("write tenant handler");
    std::fs::write(
        default_dir.join("main.phpx"),
        "function App($req: mixed) { return 'default' }",
    )
    .expect("write default handler");

    // Tenant-specific handler should be found
    let path = resolve_handler_path(&tmp, "shop_alpha");
    assert!(path.exists(), "tenant handler should exist at {:?}", path);
    assert!(
        path.to_string_lossy().contains("tenants/shop_alpha"),
        "should resolve to tenant-specific handler, got {:?}",
        path
    );

    let content = std::fs::read_to_string(&path).expect("read tenant handler");
    assert!(
        content.contains("tenant"),
        "handler should be the tenant one"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn missing_tenant_falls_back_to_default() {
    let tmp = std::env::temp_dir().join(format!(
        "deka_tenant_fallback_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let default_dir = tmp.join("default");
    std::fs::create_dir_all(&default_dir).expect("create default dir");
    std::fs::write(
        default_dir.join("main.phpx"),
        "function App($req: mixed) { return 'default' }",
    )
    .expect("write default handler");

    // Non-existent tenant should fall back to default
    let path = resolve_handler_path(&tmp, "nonexistent_shop");
    assert!(path.exists(), "should fall back to default handler");
    assert!(
        path.to_string_lossy().contains("default"),
        "should resolve to default handler, got {:?}",
        path
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn empty_shop_id_uses_default() {
    let tmp = std::env::temp_dir().join(format!(
        "deka_tenant_empty_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let default_dir = tmp.join("default");
    std::fs::create_dir_all(&default_dir).expect("create default dir");
    std::fs::write(
        default_dir.join("main.phpx"),
        "function App($req: mixed) { return 'default' }",
    )
    .expect("write default handler");

    let path = resolve_handler_path(&tmp, "");
    assert!(
        path.to_string_lossy().contains("default"),
        "empty shop_id should use default handler"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
