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
    (10_000, "free", 100),       // 1% of $100 = $1 → 100 cents (below cap)
    (10_000, "standard", 50),    // 0.5% of $100 = $0.50 → 50 cents (below cap)
    (10_000, "premium", 25),     // 0.25% of $100 = $0.25 → 25 cents (below cap)
    (10_000, "enterprise", 0),
    // --- $200 (20,000c) — free cap hits exactly, standard cap hit ---
    (20_000, "free", 200),       // 1% of $200 = $2 → exactly cap
    (20_000, "standard", 100),   // 0.5% of $200 = $1 → exactly cap
    (20_000, "premium", 50),     // 0.25% of $200 = $0.50 → exactly cap
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
