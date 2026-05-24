//! Platform → tenant environment-variable injection.
//!
//! The deka platform process inherits a set of secrets from its parent
//! environment (launchd plist, .env loader, shell). Tenant PHPX code
//! running inside a V8 isolate must NOT see arbitrary host env vars —
//! that would leak database creds, JWT secrets, third-party API keys,
//! and anything else the operator set on the host. At the same time,
//! a small, documented set of values genuinely needs to flow through:
//! Stripe publishable keys, internal server-to-server auth secrets,
//! feature flags like `STRIPE_STUB`, and so on.
//!
//! This module is the single source of truth for which platform env
//! vars are exposed to tenant isolates. The allowlist is consulted by
//! `set_request_globals` in `crates/pool/src/isolate_pool.rs` and
//! injected into `$_SERVER` alongside `SHOP_ID` / `ACCOUNT_ID`.
//!
//! ## Mechanism
//!
//! 1. A built-in default allowlist (`DEFAULT_ALLOWLIST`) covers vars
//!    the platform itself depends on. New entries land here when the
//!    platform team commits to exposing a value.
//! 2. Operators may extend the allowlist at runtime via the
//!    `DEKA_PLATFORM_ENV_ALLOWLIST` env var (comma-separated names).
//!    This lets a single launchd plist add new vars without a code
//!    change. Names are case-sensitive and must match the host env
//!    var names verbatim.
//! 3. Per-request, the runtime snapshots only allowlisted names from
//!    the platform process environment and writes the present values
//!    into `$_SERVER`. Unset vars are simply omitted — storefront
//!    code is expected to fail loud when it requires a value that is
//!    not configured (Layla's pattern: 503 if STRIPE_PUBLISHABLE_KEY
//!    is missing in prod).
//!
//! Per-tenant overrides (a tenant's own `deka.json` shadowing the
//! platform value) are intentionally **out of scope** for this
//! initial cut. The injection point is structured so adding tenant
//! overrides later is a localised change in `isolate_pool.rs`.

use std::collections::BTreeSet;

/// Built-in platform → tenant env-var allowlist.
///
/// Anything not in this list (and not in the runtime override) is
/// invisible to tenant code, even if the platform process inherited
/// it. Keep this list small and documented.
pub const DEFAULT_ALLOWLIST: &[&str] = &[
    // Stripe publishable key — passed to Stripe.js on the storefront
    // client. Public by design; safe to expose. Tenant code reads it
    // as `$_SERVER['STRIPE_PUBLISHABLE_KEY']`.
    "STRIPE_PUBLISHABLE_KEY",
    // Server-to-server auth header for storefront → store-admin
    // internal API calls (e.g. checkout/payment-intent). Sensitive,
    // but tenant storefront code MUST have it to call the platform
    // API; the network boundary is localhost-only in single-machine
    // dev and Tailscale-only in prod.
    "TANA_INTERNAL_API_SECRET",
    // Feature flag that switches the storefront checkout into a
    // local stub mode (no real Stripe round-trip). Used by tests
    // and dev environments. Boolean-shaped string ("1"/"0").
    "STRIPE_STUB",
];

/// Env-var name used to extend the allowlist at runtime.
///
/// Value is a comma-separated list of additional env-var names to
/// expose to tenant isolates. Whitespace around commas is trimmed.
/// Empty entries are ignored.
///
/// Example launchd plist snippet:
/// ```xml
/// <key>DEKA_PLATFORM_ENV_ALLOWLIST</key>
/// <string>FEATURE_FLAG_X,SOME_OTHER_PUBLIC_KEY</string>
/// ```
pub const RUNTIME_ALLOWLIST_ENV: &str = "DEKA_PLATFORM_ENV_ALLOWLIST";

/// Compute the effective allowlist (defaults + runtime override).
///
/// `env_get` is injected so callers in tests can stub out
/// `std::env::var` without depending on process-global state.
///
/// Returned names are deduplicated and stable-ordered for
/// deterministic iteration.
pub fn effective_allowlist<F>(env_get: &F) -> Vec<String>
where
    F: Fn(&str) -> Option<String>,
{
    let mut set: BTreeSet<String> = DEFAULT_ALLOWLIST.iter().map(|s| s.to_string()).collect();

    if let Some(extra) = env_get(RUNTIME_ALLOWLIST_ENV) {
        for raw in extra.split(',') {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                set.insert(trimmed.to_string());
            }
        }
    }

    set.into_iter().collect()
}

/// Convenience wrapper around [`effective_allowlist`] that reads from
/// the real process environment.
pub fn effective_allowlist_from_process() -> Vec<String> {
    effective_allowlist(&|name| std::env::var(name).ok())
}

/// Snapshot the present values of every allowlisted name from the
/// supplied env getter. Names that are unset are omitted from the
/// result (so storefront code can detect missing config and fail
/// loud rather than silently reading an empty string).
pub fn snapshot_env<F>(env_get: &F) -> Vec<(String, String)>
where
    F: Fn(&str) -> Option<String>,
{
    effective_allowlist(env_get)
        .into_iter()
        .filter_map(|name| env_get(&name).map(|value| (name, value)))
        .collect()
}

/// Convenience wrapper around [`snapshot_env`] that reads from the
/// real process environment.
pub fn snapshot_env_from_process() -> Vec<(String, String)> {
    snapshot_env(&|name| std::env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |name: &str| map.get(name).map(|v| v.to_string())
    }

    #[test]
    fn defaults_contain_stripe_and_internal_secret() {
        let names = effective_allowlist(&|_| None);
        assert!(names.iter().any(|n| n == "STRIPE_PUBLISHABLE_KEY"));
        assert!(names.iter().any(|n| n == "TANA_INTERNAL_API_SECRET"));
        assert!(names.iter().any(|n| n == "STRIPE_STUB"));
    }

    #[test]
    fn runtime_override_appends_names() {
        let env = getter(HashMap::from([(RUNTIME_ALLOWLIST_ENV, "FOO,BAR ,  BAZ ")]));
        let names = effective_allowlist(&env);
        assert!(names.iter().any(|n| n == "FOO"));
        assert!(names.iter().any(|n| n == "BAR"));
        assert!(names.iter().any(|n| n == "BAZ"));
    }

    #[test]
    fn runtime_override_does_not_drop_defaults() {
        let env = getter(HashMap::from([(RUNTIME_ALLOWLIST_ENV, "EXTRA")]));
        let names = effective_allowlist(&env);
        assert!(names.iter().any(|n| n == "STRIPE_PUBLISHABLE_KEY"));
        assert!(names.iter().any(|n| n == "EXTRA"));
    }

    #[test]
    fn empty_runtime_override_is_safe() {
        let env = getter(HashMap::from([(RUNTIME_ALLOWLIST_ENV, ",, ,")]));
        let names = effective_allowlist(&env);
        // Only defaults survive; no panics, no empty entries.
        assert_eq!(names.len(), DEFAULT_ALLOWLIST.len());
        assert!(names.iter().all(|n| !n.is_empty()));
    }

    #[test]
    fn snapshot_omits_unset_vars() {
        // Only one of the three default vars is set in this stub.
        let env = getter(HashMap::from([("STRIPE_PUBLISHABLE_KEY", "pk_test_123")]));
        let snap = snapshot_env(&env);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].0, "STRIPE_PUBLISHABLE_KEY");
        assert_eq!(snap[0].1, "pk_test_123");
    }

    #[test]
    fn snapshot_includes_runtime_override_values() {
        let env = getter(HashMap::from([
            (RUNTIME_ALLOWLIST_ENV, "EXTRA_KEY"),
            ("EXTRA_KEY", "extra_value"),
            ("STRIPE_STUB", "1"),
        ]));
        let snap = snapshot_env(&env);
        let lookup: HashMap<&str, &str> =
            snap.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(lookup.get("EXTRA_KEY").copied(), Some("extra_value"));
        assert_eq!(lookup.get("STRIPE_STUB").copied(), Some("1"));
    }

    #[test]
    fn snapshot_does_not_leak_arbitrary_env_vars() {
        // DATABASE_URL is NOT in the allowlist, so even though it's
        // set in the env, the snapshot must not surface it.
        let env = getter(HashMap::from([
            ("STRIPE_PUBLISHABLE_KEY", "pk_test"),
            ("DATABASE_URL", "postgres://secret"),
            ("AWS_SECRET_ACCESS_KEY", "shh"),
        ]));
        let snap = snapshot_env(&env);
        let names: Vec<&str> = snap.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"STRIPE_PUBLISHABLE_KEY"));
        assert!(!names.contains(&"DATABASE_URL"));
        assert!(!names.contains(&"AWS_SECRET_ACCESS_KEY"));
    }
}
