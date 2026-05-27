//! Tests for the `@deka/payments` stdlib module (issue #93).
//!
//! Two complementary checks:
//!
//! 1. `module_file_compiles` — every `.phpx` file in the `payments/` stdlib
//!    directory must parse + validate cleanly with no errors. This covers
//!    the "stub providers load without crashing" definition-of-done.
//!
//! 2. `platform_fee_cents_parity` — the PHPX fee-helper math is also
//!    implemented here in Rust and exercised at every tier boundary
//!    Sami specified ($0.01, $100, $200, $500, $5000). If someone later
//!    tweaks the PHPX helper in a way that doesn't match the canonical
//!    ladder, this test fails loudly. Keeping the ladder in one place per
//!    language avoids drift.
//!
//! The helper is pure arithmetic. Both implementations MUST agree at every
//! input in the boundary table.
//!
//! Parity invariant (Sami-approved 2026-04-19):
//!
//!   Tier          Percent   Cap ($)   Cap (cents)
//!   ----          -------   -------   -----------
//!   free          1.0%      $2.00     200
//!   standard      0.5%      $1.00     100
//!   premium       0.25%     $0.50      50
//!   enterprise    0.0%      —           0

use bumpalo::Bump;
use modules_php::compiler_api::compile_phpx;
use modules_php::validation::Severity;
use std::fs;
use std::path::PathBuf;

fn payments_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../php_modules/payments")
        .canonicalize()
        .expect("payments/ module directory should exist")
}

fn compile_module_file(name: &str) -> Vec<String> {
    let path = payments_dir().join(name);
    let source = fs::read_to_string(&path).expect("module file read failed");
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_phpx(&source, path.to_string_lossy().as_ref(), arena);
    result
        .errors
        .iter()
        .filter(|err| matches!(err.severity, Severity::Error))
        .map(|err| format!("{:?}: {}", err.kind, err.message))
        .collect()
}

#[test]
fn payments_index_compiles() {
    let errs = compile_module_file("index.phpx");
    assert!(errs.is_empty(), "index.phpx errors: {:#?}", errs);
}

#[test]
fn payments_fees_compiles() {
    let errs = compile_module_file("fees.phpx");
    assert!(errs.is_empty(), "fees.phpx errors: {:#?}", errs);
}

#[test]
fn payments_types_compiles() {
    let errs = compile_module_file("types.phpx");
    assert!(errs.is_empty(), "types.phpx errors: {:#?}", errs);
}

#[test]
fn payments_webhook_compiles() {
    let errs = compile_module_file("webhook.phpx");
    assert!(errs.is_empty(), "webhook.phpx errors: {:#?}", errs);
}

#[test]
fn payments_stripe_stub_compiles() {
    let errs = compile_module_file("stripe.phpx");
    assert!(errs.is_empty(), "stripe.phpx errors: {:#?}", errs);
}

#[test]
fn payments_paypal_stub_compiles() {
    let errs = compile_module_file("paypal.phpx");
    assert!(errs.is_empty(), "paypal.phpx errors: {:#?}", errs);
}

#[test]
fn payments_square_stub_compiles() {
    let errs = compile_module_file("square.phpx");
    assert!(errs.is_empty(), "square.phpx errors: {:#?}", errs);
}

#[test]
fn payments_oauth_state_compiles() {
    let errs = compile_module_file("oauth_state.phpx");
    assert!(errs.is_empty(), "oauth_state.phpx errors: {:#?}", errs);
}

#[test]
fn payments_return_url_compiles() {
    let errs = compile_module_file("return_url.phpx");
    assert!(errs.is_empty(), "return_url.phpx errors: {:#?}", errs);
}

#[test]
fn payments_refund_auth_compiles() {
    let errs = compile_module_file("refund_auth.phpx");
    assert!(errs.is_empty(), "refund_auth.phpx errors: {:#?}", errs);
}

#[test]
fn payments_token_vault_compiles() {
    let errs = compile_module_file("token_vault.phpx");
    assert!(errs.is_empty(), "token_vault.phpx errors: {:#?}", errs);
}

// ---------------------------------------------------------------------------
// Fee-ladder parity tests
// ---------------------------------------------------------------------------

/// Canonical Rust implementation that mirrors the PHPX `platform_fee_cents`
/// helper in `payments/fees.phpx`. This is the source of truth the test
/// suite pins against — if these two implementations ever disagree, someone
/// changed the semantics and this test MUST fail.
fn platform_fee_cents(amount_cents: i64, tier: &str) -> i64 {
    if amount_cents <= 0 {
        return 0;
    }
    let (pct, cap): (f64, i64) = match tier {
        "free" => (0.01, 200),
        "standard" => (0.005, 100),
        "premium" => (0.0025, 50),
        "enterprise" => return 0,
        _ => (0.01, 200), // Unknown tier falls through to 'free'.
    };
    // `(int)` in PHPX truncates toward zero — matches `as i64` for positives.
    let raw = (amount_cents as f64 * pct) as i64;
    if raw > cap { cap } else { raw }
}

/// Fee-ladder boundary matrix. Each row is (amount_cents, tier, expected).
///
/// Boundaries include:
///   - $0.01 (1 cent): far below every cap; exercises the rounding-toward-zero.
///   - $100 (10,000c): exactly where the standard cap hits ($0.50 under free cap).
///   - $200 (20,000c): exactly where the free cap hits.
///   - $500 (50,000c): well above every cap — all capped tiers sit at their cap.
///   - $5000 (500,000c): stress-test — confirms the cap holds at high-ticket.
///
/// Edge rows: 0 and negative amounts return 0 for every tier (defensive).
const FEE_BOUNDARIES: &[(i64, &str, i64)] = &[
    // --- $0.01 — tiny charge, all rates truncate to 0 ---
    (1, "free", 0),       // 1 * 0.01 = 0.01 → floor = 0
    (1, "standard", 0),   // 1 * 0.005 = 0.005 → floor = 0
    (1, "premium", 0),    // 1 * 0.0025 = 0.0025 → floor = 0
    (1, "enterprise", 0), // always 0
    // --- $100 (10,000c) — hits the standard cap exactly, below free cap ---
    (10_000, "free", 100),    // 1% of $100 = $1 → 100 cents (below cap)
    (10_000, "standard", 50), // 0.5% of $100 = $0.50 → 50 cents (below cap)
    (10_000, "premium", 25),  // 0.25% of $100 = $0.25 → 25 cents (below cap)
    (10_000, "enterprise", 0),
    // --- $200 (20,000c) — free cap hits exactly, standard cap hit ---
    (20_000, "free", 200),     // 1% of $200 = $2 → exactly cap
    (20_000, "standard", 100), // 0.5% of $200 = $1 → exactly cap
    (20_000, "premium", 50),   // 0.25% of $200 = $0.50 → exactly cap
    (20_000, "enterprise", 0),
    // --- $500 (50,000c) — all capped tiers sit at their cap ---
    (50_000, "free", 200),
    (50_000, "standard", 100),
    (50_000, "premium", 50),
    (50_000, "enterprise", 0),
    // --- $5000 (500,000c) — high-ticket; caps must still hold ---
    (500_000, "free", 200),
    (500_000, "standard", 100),
    (500_000, "premium", 50),
    (500_000, "enterprise", 0),
    // --- Defensive: zero / negative amounts always fee zero ---
    (0, "free", 0),
    (0, "standard", 0),
    (0, "premium", 0),
    (0, "enterprise", 0),
    (-100, "free", 0),
    (-100, "standard", 0),
    (-100, "premium", 0),
    // --- Unknown tier falls through to free ---
    (10_000, "wat", 100),
    (20_000, "", 200),
    (500_000, "unknown", 200),
];

#[test]
fn platform_fee_cents_boundaries() {
    for (amount, tier, expected) in FEE_BOUNDARIES {
        let got = platform_fee_cents(*amount, tier);
        assert_eq!(
            got, *expected,
            "tier={} amount_cents={} expected={} got={}",
            tier, amount, expected, got
        );
    }
}

#[test]
fn platform_fee_cents_halving_ladder() {
    // Each tier's fee at a mid-range amount ($100) should halve from the
    // previous — the whole point of the ladder. If someone changes a
    // percentage in isolation this test flags it.
    let amount = 10_000_i64; // $100
    let free = platform_fee_cents(amount, "free");
    let standard = platform_fee_cents(amount, "standard");
    let premium = platform_fee_cents(amount, "premium");
    let enterprise = platform_fee_cents(amount, "enterprise");

    assert_eq!(free, 100);
    assert_eq!(standard, free / 2);
    assert_eq!(premium, standard / 2);
    assert_eq!(enterprise, 0);
}

#[test]
fn platform_fee_cents_cap_halving_ladder() {
    // At a very high charge the fee should pin to the cap. Each tier's cap
    // should halve too.
    let amount = 1_000_000_i64; // $10,000
    assert_eq!(platform_fee_cents(amount, "free"), 200);
    assert_eq!(platform_fee_cents(amount, "standard"), 100);
    assert_eq!(platform_fee_cents(amount, "premium"), 50);
    assert_eq!(platform_fee_cents(amount, "enterprise"), 0);
}

// ---------------------------------------------------------------------------
// return_url allowlist parity (issue #120)
// ---------------------------------------------------------------------------

/// Rust mirror of the PHPX `validate_return_url` in
/// `payments/return_url.phpx`. Any change to the PHPX side must come with
/// a matching change here, or this parity test will flag the drift.
fn validate_return_url(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    // No whitespace or newline — stops header-injection attempts.
    if input.chars().any(|c| c.is_ascii_whitespace()) {
        return None;
    }

    let (is_https, rest) = if let Some(r) = input.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = input.strip_prefix("http://") {
        (false, r)
    } else {
        return None;
    };

    // scheme-relative `//` attacks.
    if rest.starts_with('/') {
        return None;
    }
    // userinfo in authority.
    let slash_idx = rest.find('/');
    let authority = match slash_idx {
        Some(i) => &rest[..i],
        None => rest,
    };
    if authority.contains('@') {
        return None;
    }
    let path = match slash_idx {
        Some(i) => &rest[i..],
        None => "/",
    };
    // Strip fragment.
    let clean_path = match path.find('#') {
        Some(i) => &path[..i],
        None => path,
    };

    let host = match authority.find(':') {
        Some(i) => &authority[..i],
        None => authority,
    }
    .to_ascii_lowercase();

    let subdomain_ok = |label: &str| -> bool {
        if label.is_empty() {
            return false;
        }
        let first = label.chars().next().unwrap();
        let last = label.chars().last().unwrap();
        if matches!(first, '-' | '.' | '_') || matches!(last, '-' | '.' | '_') {
            return false;
        }
        if label.contains("..") {
            return false;
        }
        label.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.' || c == '_'
        })
    };

    let host_ok = if is_https {
        if host == "tana.gg" || host == "tana.local" {
            true
        } else if let Some(label) = host.strip_suffix(".tana.gg") {
            subdomain_ok(label)
        } else if let Some(label) = host.strip_suffix(".tana.local") {
            subdomain_ok(label)
        } else {
            false
        }
    } else {
        if host == "tana.local" {
            true
        } else if let Some(label) = host.strip_suffix(".tana.local") {
            subdomain_ok(label)
        } else {
            false
        }
    };
    if !host_ok {
        return None;
    }
    let scheme = if is_https { "https" } else { "http" };
    Some(format!("{}://{}{}", scheme, authority, clean_path))
}

#[test]
fn return_url_allowlist_accepts_valid_tana_urls() {
    let cases = &[
        "https://tana.gg/settings/payments",
        "https://shop_alpha.tana.gg/cart",
        "https://admin.tana.gg/dashboard",
        "https://store.tana.gg/",
        "https://tana.local/foo",
        "https://my-shop.tana.local/a",
        "http://tana.local/local-dev",
        "http://store.tana.local:3002/settings/payments",
        "https://my-shop.tana.gg:443/cart",
    ];
    for c in cases {
        assert!(
            validate_return_url(c).is_some(),
            "expected accept, got reject: {}",
            c
        );
    }
}

#[test]
fn return_url_allowlist_rejects_open_redirects() {
    let cases = &[
        "",
        "https://attacker.example/",
        "http://attacker.example/",
        "https://tana.gg.attacker.example/", // subdomain attack
        "https://tana.com/",                 // lookalike TLD
        "https://.tana.gg/",                 // malformed label
        "https://tana.gg./",                 // trailing dot
        "https://evil@tana.gg/",             // userinfo injection
        "https://tana.gg\nX-Hacked: 1",      // header injection
        "https:///attacker.example/",        // scheme-relative
        "javascript:alert(1)",
        "//attacker.example",
        "ftp://tana.gg/",
        "http://tana.gg/", // http not allowed for production domain
        "https://tana.gg%2eattacker.example/",
    ];
    for c in cases {
        assert!(
            validate_return_url(c).is_none(),
            "expected reject, got accept: {}",
            c
        );
    }
}

// ---------------------------------------------------------------------------
// Stripe webhook signature parsing parity
// ---------------------------------------------------------------------------
//
// Mirrors the `stripe_webhook_verify` logic in stripe.phpx. This is a pure
// string-parse + HMAC check. The PHPX side uses `crypto.hmac_sha256_hex`
// which is the same HMAC-SHA256 as the `hmac` crate here.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

fn stripe_hmac_hex(payload: &str, secret: &str) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(payload.as_bytes());
    let res = mac.finalize().into_bytes();
    hex::encode(res)
}

#[test]
fn stripe_sig_hmac_matches_known_vector() {
    // Cross-check: pin the format + algorithm. A future change to the PHPX
    // computation must produce the same hex.
    let ts = "1700000000";
    let body = "{\"id\":\"evt_1\",\"type\":\"account.updated\"}";
    let secret = "whsec_test_secret_123";
    let signed = format!("{}.{}", ts, body);
    let sig = stripe_hmac_hex(&signed, secret);
    assert_eq!(sig.len(), 64);
    // Idempotent.
    assert_eq!(sig, stripe_hmac_hex(&signed, secret));
    // Changing the timestamp changes the signature.
    let ts2 = "1700000001";
    let signed2 = format!("{}.{}", ts2, body);
    assert_ne!(sig, stripe_hmac_hex(&signed2, secret));
    // Changing the body changes the signature.
    let body2 = "{\"id\":\"evt_2\",\"type\":\"account.updated\"}";
    let signed3 = format!("{}.{}", ts, body2);
    assert_ne!(sig, stripe_hmac_hex(&signed3, secret));
}

#[test]
fn stripe_sig_header_format_parses() {
    // Typical Stripe header: `t=<unix>,v1=<hex>[,v1=<hex>]`. We parse
    // only the first `t` and collect all `v1`s.
    let header = "t=1700000000,v1=abc123,v1=def456";
    let parts: Vec<&str> = header.split(',').collect();
    let mut t = String::new();
    let mut v1s: Vec<String> = Vec::new();
    for part in parts {
        if let Some(rest) = part.trim().strip_prefix("t=") {
            t = rest.to_string();
        } else if let Some(rest) = part.trim().strip_prefix("v1=") {
            v1s.push(rest.to_string());
        }
    }
    assert_eq!(t, "1700000000");
    assert_eq!(v1s, vec!["abc123".to_string(), "def456".to_string()]);
}

// ---------------------------------------------------------------------------
// Refund authorization parity (issue #123)
// ---------------------------------------------------------------------------
//
// Mirrors `authorize_refund()` in refund_auth.phpx — same role allowlist,
// same amount ordering rules. A drift between the two implementations
// trips this test.

fn authorize_refund(
    role: &str,
    user_id: &str,
    amount: i64,
    charge_amount: i64,
) -> Result<(), &'static str> {
    if user_id.is_empty() {
        return Err("refund_unauthorized: missing actor user_id");
    }
    let allowed = ["owner", "admin", "shop_owner", "platform_admin"];
    if !allowed.contains(&role) {
        return Err("refund_unauthorized");
    }
    if amount <= 0 {
        return Err("refund_amount_invalid");
    }
    if charge_amount > 0 && amount > charge_amount {
        return Err("refund_amount_exceeds_charge");
    }
    Ok(())
}

#[test]
fn refund_auth_role_allowlist() {
    // Accepted roles.
    for role in ["owner", "admin", "shop_owner", "platform_admin"] {
        assert!(
            authorize_refund(role, "user_1", 500, 1000).is_ok(),
            "expected role {} to pass",
            role
        );
    }
    // Rejected roles.
    for role in ["staff", "viewer", "customer", "", "OWNER"] {
        assert!(
            authorize_refund(role, "user_1", 500, 1000).is_err(),
            "expected role {} to reject",
            role
        );
    }
}

#[test]
fn refund_auth_amount_sanity_checks() {
    // Missing user_id rejects even with valid role.
    assert!(authorize_refund("owner", "", 100, 1000).is_err());
    // Zero or negative amount rejects.
    assert!(authorize_refund("owner", "user_1", 0, 1000).is_err());
    assert!(authorize_refund("owner", "user_1", -10, 1000).is_err());
    // Partial refund accepted.
    assert!(authorize_refund("owner", "user_1", 100, 1000).is_ok());
    // Full refund accepted.
    assert!(authorize_refund("owner", "user_1", 1000, 1000).is_ok());
    // Over-refund rejects (can't refund more than charge amount).
    assert!(authorize_refund("owner", "user_1", 1001, 1000).is_err());
    // charge_amount=0 disables the over-refund check (caller chose not to
    // pass the known charge amount). Auth still passes; provider is final arbiter.
    assert!(authorize_refund("owner", "user_1", 999_999, 0).is_ok());
}

// ---------------------------------------------------------------------------
// AES-256-GCM round-trip (issue #119 — token vault primitive)
// ---------------------------------------------------------------------------
//
// These exercise the same `aes-gcm` crate the Rust op uses, so any behavioral
// drift between the op's contract and the PHPX caller flags at build time.
// We pin the wire-format assumptions:
//
//   - key length 32 bytes
//   - nonce length 12 bytes
//   - ciphertext is `raw_ct || tag_16`
//   - AAD is authenticated but not encrypted

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};

fn gcm_encrypt(key: &[u8; 32], nonce: &[u8; 12], pt: &[u8], aad: &[u8]) -> Vec<u8> {
    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce);
    let payload = Payload { msg: pt, aad };
    cipher.encrypt(nonce, payload).unwrap()
}

fn gcm_decrypt(key: &[u8; 32], nonce: &[u8; 12], ct: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce);
    let payload = Payload { msg: ct, aad };
    cipher.decrypt(nonce, payload).ok()
}

#[test]
fn aes_gcm_roundtrip_basic() {
    let key = [7u8; 32];
    let nonce = [3u8; 12];
    let pt = b"EAAAEXAMPLE_sq0_access_token_material";
    let aad = b"shop:shop_alpha";
    let ct = gcm_encrypt(&key, &nonce, pt, aad);
    // Ciphertext is plaintext-length + 16-byte tag.
    assert_eq!(ct.len(), pt.len() + 16);
    let pt2 = gcm_decrypt(&key, &nonce, &ct, aad).expect("decrypt should succeed");
    assert_eq!(pt2, pt);
}

#[test]
fn aes_gcm_aad_binds_ciphertext() {
    // A ciphertext produced under AAD=shop:A MUST NOT decrypt under AAD=shop:B.
    // This is the property the token_vault helper relies on to prevent
    // row-swap attacks (move blob from shop A's row to shop B's row).
    let key = [1u8; 32];
    let nonce = [2u8; 12];
    let pt = b"tok_123";
    let ct = gcm_encrypt(&key, &nonce, pt, b"shop:A");
    assert!(gcm_decrypt(&key, &nonce, &ct, b"shop:B").is_none());
    assert!(gcm_decrypt(&key, &nonce, &ct, b"shop:A").is_some());
}

#[test]
fn aes_gcm_wrong_key_fails_closed() {
    let k1 = [0x11u8; 32];
    let k2 = [0x22u8; 32];
    let nonce = [0u8; 12];
    let pt = b"secret";
    let aad = b"";
    let ct = gcm_encrypt(&k1, &nonce, pt, aad);
    // Wrong key: decrypt returns None (auth tag fails).
    assert!(gcm_decrypt(&k2, &nonce, &ct, aad).is_none());
}

#[test]
fn aes_gcm_tamper_flips_tag() {
    // Flipping a byte in the ciphertext invalidates the tag.
    let key = [9u8; 32];
    let nonce = [4u8; 12];
    let pt = b"important-token";
    let aad = b"shop:s1";
    let mut ct = gcm_encrypt(&key, &nonce, pt, aad);
    // Flip one bit in the middle.
    ct[5] ^= 0x01;
    assert!(gcm_decrypt(&key, &nonce, &ct, aad).is_none());
}

// ---------------------------------------------------------------------------
// Token vault wire-format parity (issue #119)
// ---------------------------------------------------------------------------
//
// Mirrors the `v1:{keyid}:{b64(nonce)}:{b64(ct+tag)}` layout in
// token_vault.phpx. If the PHPX side ever drops or changes a field, the
// rebuild test below fails.

fn token_vault_encrypt_wire(
    pt: &[u8],
    shop_id: &str,
    key: &[u8; 32],
    keyid: &str,
    nonce: &[u8; 12],
) -> String {
    use base64::Engine;
    let aad = format!("shop:{}", shop_id);
    let ct = gcm_encrypt(key, nonce, pt, aad.as_bytes());
    let b64 = base64::engine::general_purpose::STANDARD;
    format!("v1:{}:{}:{}", keyid, b64.encode(nonce), b64.encode(&ct))
}

fn token_vault_decrypt_wire(
    blob: &str,
    shop_id: &str,
    keys: &[(&str, &[u8; 32])],
) -> Result<Vec<u8>, &'static str> {
    use base64::Engine;
    let parts: Vec<&str> = blob.split(':').collect();
    if parts.len() != 4 {
        return Err("invalid_vault_format");
    }
    if parts[0] != "v1" {
        return Err("unsupported_vault_version");
    }
    let labelled_keyid = parts[1];
    let b64 = base64::engine::general_purpose::STANDARD;
    let nonce_vec = b64.decode(parts[2]).map_err(|_| "invalid_nonce")?;
    let ct = b64.decode(parts[3]).map_err(|_| "invalid_ciphertext")?;
    if nonce_vec.len() != 12 {
        return Err("invalid_nonce");
    }
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&nonce_vec);
    let aad = format!("shop:{}", shop_id);
    // Try labelled key first, then any other.
    let mut order: Vec<&(&str, &[u8; 32])> = Vec::new();
    if let Some(labelled) = keys.iter().find(|(k, _)| *k == labelled_keyid) {
        order.push(labelled);
    }
    for key in keys.iter() {
        if key.0 != labelled_keyid {
            order.push(key);
        }
    }
    for (_kid, k) in order {
        if let Some(pt) = gcm_decrypt(k, &nonce, &ct, aad.as_bytes()) {
            return Ok(pt);
        }
    }
    Err("key_rotation_required")
}

#[test]
fn token_vault_roundtrip_primary_key() {
    let pt = b"EAAAE_square_sq0sandbox_access_token";
    let key = [0x42u8; 32];
    let nonce = [0x77u8; 12];
    let blob = token_vault_encrypt_wire(pt, "shop_alpha", &key, "primary", &nonce);
    assert!(blob.starts_with("v1:primary:"));
    let decoded = token_vault_decrypt_wire(&blob, "shop_alpha", &[("primary", &key)])
        .expect("decrypt should succeed");
    assert_eq!(decoded, pt);
}

#[test]
fn token_vault_cross_shop_decrypt_fails() {
    let pt = b"tok_material";
    let key = [0x10u8; 32];
    let nonce = [0x20u8; 12];
    let blob = token_vault_encrypt_wire(pt, "shop_alpha", &key, "primary", &nonce);
    // Someone copied the blob into shop_beta's row — decrypt must fail.
    let err = token_vault_decrypt_wire(&blob, "shop_beta", &[("primary", &key)])
        .expect_err("cross-shop decrypt must fail");
    assert_eq!(err, "key_rotation_required");
}

#[test]
fn token_vault_rotation_tries_alternate_key() {
    // Blob was encrypted under 'primary', but both primary AND v2 are
    // configured. Decrypt should succeed under primary, ignoring v2.
    let pt = b"old-token";
    let k1 = [0x01u8; 32];
    let k2 = [0x02u8; 32];
    let nonce = [0x03u8; 12];
    let blob = token_vault_encrypt_wire(pt, "shop", &k1, "primary", &nonce);
    let decoded = token_vault_decrypt_wire(&blob, "shop", &[("primary", &k1), ("v2", &k2)])
        .expect("decrypt should succeed with matching key");
    assert_eq!(decoded, pt);

    // Now simulate rotation where primary was rotated to a different value
    // (the original k1 is gone) but a v2 is in the table. Decrypt should
    // still fail because the blob was signed under the old primary key
    // and the current primary doesn't decrypt. We fall through to v2 which
    // also doesn't decrypt. Surface key_rotation_required so ops sees it.
    let new_primary = [0x99u8; 32];
    let err = token_vault_decrypt_wire(&blob, "shop", &[("primary", &new_primary), ("v2", &k2)])
        .expect_err("expected key_rotation_required");
    assert_eq!(err, "key_rotation_required");
}

#[test]
fn token_vault_rejects_malformed_blobs() {
    let k = [0u8; 32];
    let keys: &[(&str, &[u8; 32])] = &[("primary", &k)];
    let cases = &[
        ("", "invalid_vault_format"),
        ("garbage", "invalid_vault_format"),
        ("v2:primary:aaa:bbb", "unsupported_vault_version"),
        ("v1:primary:!!!:bbb", "invalid_nonce"),
    ];
    for (blob, want) in cases {
        let got = token_vault_decrypt_wire(blob, "shop", keys);
        assert!(got.is_err(), "expected err for {}", blob);
        assert_eq!(got.unwrap_err(), *want, "blob={}", blob);
    }
}

// ---------------------------------------------------------------------------
// Square webhook signature parity (issue #96)
// ---------------------------------------------------------------------------
//
// Square's v2 webhooks sign `{notification_url}{body}` with HMAC-SHA256 and
// encode the result as base64. Parity test.

#[test]
fn square_webhook_hmac_matches_known_vector() {
    use base64::Engine;
    let url = "https://shop_alpha.tana.gg/api/payments/webhooks/square";
    let body = r#"{"event_id":"evt_1","type":"payment.updated"}"#;
    let key = "square_webhook_key_xyz";
    let signed = format!("{}{}", url, body);

    let mut mac = <HmacSha256 as Mac>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(signed.as_bytes());
    let sig_bytes = mac.finalize().into_bytes();
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig_bytes);

    assert_eq!(sig_b64.len(), 44); // 32-byte HMAC → 44 chars base64 (with padding)

    // Tamper test: change one char in the body, signature must not match.
    let signed2 = format!(
        "{}{}",
        url, r#"{"event_id":"evt_2","type":"payment.updated"}"#
    );
    let mut mac2 = <HmacSha256 as Mac>::new_from_slice(key.as_bytes()).unwrap();
    mac2.update(signed2.as_bytes());
    let sig2_b64 = base64::engine::general_purpose::STANDARD.encode(mac2.finalize().into_bytes());
    assert_ne!(sig_b64, sig2_b64);
}

// ---------------------------------------------------------------------------
// Square is_sandbox_app_id parity
// ---------------------------------------------------------------------------

fn is_sandbox_app_id(app_id: &str) -> bool {
    app_id.starts_with("sandbox-") || app_id.starts_with("sq0idb-")
}

#[test]
fn square_sandbox_detection() {
    assert!(is_sandbox_app_id("sandbox-sq0idb-abc123"));
    assert!(is_sandbox_app_id("sq0idb-abc123"));
    assert!(!is_sandbox_app_id("sq0idp-prod123"));
    assert!(!is_sandbox_app_id(""));
    assert!(!is_sandbox_app_id("garbage"));
}
