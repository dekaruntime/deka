//! Platform → tenant environment-variable injection.
//!
//! Tenant PHPX code must not see arbitrary host env vars. The runtime
//! snapshots only names allowed by the resolved `deka.json` security
//! policy and injects those values into `$_SERVER`, `$_ENV`, and
//! `process.env`. Missing `security.allow.env` fails closed.

use std::collections::BTreeSet;

use crate::security_policy::{RuleList, SecurityPolicy, parse_deka_security_policy};

/// Snapshot the present values of every manifest-allowed name from the
/// supplied env getter. Names that are unset are omitted from the
/// result (so storefront code can detect missing config and fail
/// loud rather than silently reading an empty string).
pub fn snapshot_env<F>(policy: &SecurityPolicy, env_get: &F) -> Vec<(String, String)>
where
    F: Fn(&str) -> Option<String>,
{
    effective_env_allowlist(policy, env_get)
        .into_iter()
        .filter_map(|name| env_get(&name).map(|value| (name, value)))
        .collect()
}

pub fn effective_env_allowlist<F>(policy: &SecurityPolicy, _env_get: &F) -> Vec<String>
where
    F: Fn(&str) -> Option<String>,
{
    let mut set = match &policy.allow.env {
        RuleList::None => BTreeSet::new(),
        RuleList::All => std::env::vars().map(|(name, _)| name).collect(),
        RuleList::List(items) => items.iter().cloned().collect(),
    };

    match &policy.deny.env {
        RuleList::None => {}
        RuleList::All => set.clear(),
        RuleList::List(items) => {
            for item in items {
                set.remove(item);
            }
        }
    }

    set.into_iter().collect()
}

pub fn snapshot_env_from_security_policy_env<F>(env_get: &F) -> Vec<(String, String)>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = env_get("DEKA_SECURITY_POLICY") else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let parsed = parse_deka_security_policy(&json);
    if parsed.has_errors() {
        return Vec::new();
    }
    snapshot_env(&parsed.policy, env_get)
}

/// Convenience wrapper around [`snapshot_env_from_security_policy_env`]
/// that reads from the real process environment.
pub fn snapshot_env_from_process() -> Vec<(String, String)> {
    snapshot_env_from_security_policy_env(&|name| std::env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |name: &str| map.get(name).map(|v| v.to_string())
    }

    fn policy(allow: serde_json::Value, deny: serde_json::Value) -> SecurityPolicy {
        let parsed = parse_deka_security_policy(&serde_json::json!({
            "security": { "allow": { "env": allow }, "deny": { "env": deny } }
        }));
        assert!(!parsed.has_errors());
        parsed.policy
    }

    #[test]
    fn snapshot_omits_unset_manifest_vars() {
        let env = getter(HashMap::from([("STRIPE_PUBLISHABLE_KEY", "pk_test_123")]));
        let snap = snapshot_env(
            &policy(
                serde_json::json!(["STRIPE_PUBLISHABLE_KEY", "MISSING"]),
                serde_json::json!(false),
            ),
            &env,
        );
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].0, "STRIPE_PUBLISHABLE_KEY");
        assert_eq!(snap[0].1, "pk_test_123");
    }

    #[test]
    fn snapshot_includes_only_manifest_allowed_values() {
        let env = getter(HashMap::from([
            ("EXTRA_KEY", "extra_value"),
            ("STRIPE_TEST_ONLY", "1"),
        ]));
        let snap = snapshot_env(
            &policy(serde_json::json!(["EXTRA_KEY"]), serde_json::json!(false)),
            &env,
        );
        let lookup: HashMap<&str, &str> =
            snap.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(lookup.get("EXTRA_KEY").copied(), Some("extra_value"));
        assert_eq!(lookup.get("STRIPE_TEST_ONLY").copied(), None);
    }

    #[test]
    fn snapshot_does_not_leak_arbitrary_env_vars_when_env_missing() {
        let env = getter(HashMap::from([
            ("STRIPE_PUBLISHABLE_KEY", "pk_test"),
            ("DATABASE_URL", "postgres://secret"),
            ("AWS_SECRET_ACCESS_KEY", "shh"),
        ]));
        let snap = snapshot_env(&SecurityPolicy::default(), &env);
        assert!(snap.is_empty());
    }

    #[test]
    fn deny_env_removes_allowed_name() {
        let env = getter(HashMap::from([("PUBLIC", "ok"), ("SECRET", "no")]));
        let snap = snapshot_env(
            &policy(
                serde_json::json!(["PUBLIC", "SECRET"]),
                serde_json::json!(["SECRET"]),
            ),
            &env,
        );
        let names: Vec<&str> = snap.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["PUBLIC"]);
    }

    #[test]
    fn snapshot_reads_deka_security_policy_from_env() {
        let env = getter(HashMap::from([
            (
                "DEKA_SECURITY_POLICY",
                r#"{"security":{"allow":{"env":["PUBLIC"]},"deny":{}}}"#,
            ),
            ("PUBLIC", "ok"),
            ("SECRET", "no"),
        ]));
        let snap = snapshot_env_from_security_policy_env(&env);
        assert_eq!(snap, vec![("PUBLIC".to_string(), "ok".to_string())]);
    }
}
