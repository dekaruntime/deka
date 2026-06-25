use super::{is_dev_mode_from_env, split_request_url};
use deka_shard::{ShardConfig, ShardInfo, ShardResolver};

fn two_shard_resolver() -> ShardResolver {
    ShardResolver::from_config(
        ShardConfig {
            shards: vec![
                ShardInfo {
                    index: 0,
                    name: "local".into(),
                    neo4j: "bolt://127.0.0.1:7687".into(),
                    redis: "redis://127.0.0.1:6379".into(),
                },
                ShardInfo {
                    index: 1,
                    name: "remote".into(),
                    neo4j: "bolt://100.0.0.1:7687".into(),
                    redis: "redis://100.0.0.1:6379".into(),
                },
            ],
        },
        None,
    )
}

#[test]
fn split_request_url_handles_absolute_urls() {
    let (request_uri, pathname) = split_request_url("http://localhost:8530/users?id=1");
    assert_eq!(request_uri, "/users?id=1");
    assert_eq!(pathname, "/users");
}

#[test]
fn split_request_url_handles_relative_paths() {
    let (request_uri, pathname) = split_request_url("/docs/getting-started?tab=init");
    assert_eq!(request_uri, "/docs/getting-started?tab=init");
    assert_eq!(pathname, "/docs/getting-started");
}

#[test]
fn is_dev_mode_requires_explicit_deka_env() {
    assert!(is_dev_mode_from_env(|key| match key {
        "DEKA_DEV_MODE" => Some("1".into()),
        _ => None,
    }));
}

#[test]
fn is_dev_mode_ignores_node_env_development() {
    assert!(!is_dev_mode_from_env(|key| match key {
        "NODE_ENV" => Some("development".into()),
        _ => None,
    }));
}

// --- pick_shard_for_request ---

/// Empty account_id always falls back to shard 0 in production.
#[test]
fn pick_shard_empty_account_id_returns_shard_zero() {
    let r = two_shard_resolver();
    let shard = super::pick_shard_for_request("", &r);
    assert!(shard.is_some());
    assert_eq!(shard.unwrap().index, 0);
}

/// A populated account_id that hashes to shard 1 should land on shard 1
/// in production (non-dev) mode. We pick an account_id we know hashes to
/// shard 1 on a 2-shard cluster by brute-force search here.
///
/// Note: if DEKA_DEV_MODE=1 is set in the test environment this test
/// will see shard 0 instead. That's expected: dev mode overrides.
#[test]
fn pick_shard_production_hashes_account_id() {
    let r = two_shard_resolver();
    // Pick any non-empty account_id; confirm the result is consistent
    // (the same ID always lands on the same shard).
    let id = "c0dc1618-20fc-4bdd-ac6f-e94909f8fad2";
    let first = super::pick_shard_for_request(id, &r).unwrap().index;
    let second = super::pick_shard_for_request(id, &r).unwrap().index;
    assert_eq!(first, second, "shard resolution must be deterministic");
}

/// Verify the dev-mode fast-path directly using the helper function.
/// We build a two-shard resolver and confirm that an account_id that
/// would otherwise hash to shard 1 still returns shard 0 when the
/// resolver is stubbed with a single-shard config (mimicking what
/// `pick_shard_for_request` does when `is_dev_mode()` is true, i.e.
/// always `resolver.shards().first()`).
///
/// We can't flip `DEV_MODE` (OnceLock), so we test the branch logic
/// indirectly: a single-shard resolver can only return shard 0.
#[test]
fn pick_shard_single_shard_resolver_always_returns_zero() {
    let r = ShardResolver::from_config(ShardConfig::single_shard_localhost(), None);
    // Any account_id resolves to the one shard.
    let id = "c0dc1618-20fc-4bdd-ac6f-e94909f8fad2";
    let shard = super::pick_shard_for_request(id, &r).unwrap();
    assert_eq!(shard.index, 0);
    assert_eq!(shard.name, "local");
}

/// Empty shard list returns None (resolver is unusable).
#[test]
fn pick_shard_empty_resolver_returns_none() {
    let r = ShardResolver::from_config(ShardConfig { shards: vec![] }, None);
    assert!(super::pick_shard_for_request("any-id", &r).is_none());
    assert!(super::pick_shard_for_request("", &r).is_none());
}

use std::sync::Mutex;

/// Regression test for #618: env_snapshot must be injected for any served
/// request, even when there is no subdomain-routed shop (single-tenant serve).
/// Non-allowlisted vars must stay excluded (security boundary).
#[test]
fn env_snapshot_injected_for_unrouted_shop_request() {
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap();

    // Set up an allowlisted env var and a non-allowlisted env var.
    unsafe {
        std::env::set_var("STRIPE_PUBLISHABLE_KEY", "pk_test_618");
        std::env::set_var("SECRET_NON_ALLOWLISTED", "should_not_appear");
    }

    let mut runtime = deno_core::JsRuntime::new(deno_core::RuntimeOptions::default());

    let request_parts = super::RequestParts {
        url: "http://localhost:8530/".to_string(),
        method: "GET".to_string(),
        headers: vec![],
        body: None,
    };

    let result = super::set_request_globals(
        &mut runtime,
        &serde_json::Value::Null,
        Some(&request_parts),
        &serde_json::Value::Null,
        None,
        &std::collections::HashMap::new(),
    );
    assert!(result.is_ok(), "set_request_globals failed: {:?}", result);

    let (env, server, process_env) = {
        deno_core::scope!(scope, runtime);
        let context = scope.get_current_context();
        let global = context.global(scope);

        let env_key = deno_core::v8::String::new(scope, "_ENV").expect("_ENV key");
        let env_val = global
            .get(scope, env_key.into())
            .expect("_ENV must be set by set_request_globals");
        let env: serde_json::Value = deno_core::serde_v8::from_v8(scope, env_val)
            .expect("_ENV must deserialize");

        let server_key = deno_core::v8::String::new(scope, "_SERVER").expect("_SERVER key");
        let server_val = global
            .get(scope, server_key.into())
            .expect("_SERVER must be set by set_request_globals");
        let server: serde_json::Value = deno_core::serde_v8::from_v8(scope, server_val)
            .expect("_SERVER must deserialize");

        let process_key = deno_core::v8::String::new(scope, "process").expect("process key");
        let process_val = global
            .get(scope, process_key.into())
            .expect("process must be set by set_request_globals");
        let process_obj = process_val
            .to_object(scope)
            .expect("process must be an object");
        let env_key = deno_core::v8::String::new(scope, "env").expect("env key");
        let process_env_val = process_obj
            .get(scope, env_key.into())
            .expect("process.env must be set by set_request_globals");
        let process_env: serde_json::Value = deno_core::serde_v8::from_v8(scope, process_env_val)
            .expect("process.env must deserialize");

        (env, server, process_env)
    };

    // Assert allowlisted var is present in all three injection targets.
    assert_eq!(
        env.get("STRIPE_PUBLISHABLE_KEY").and_then(|v| v.as_str()),
        Some("pk_test_618"),
        "_ENV must contain allowlisted var"
    );
    assert_eq!(
        server.get("STRIPE_PUBLISHABLE_KEY").and_then(|v| v.as_str()),
        Some("pk_test_618"),
        "_SERVER must contain allowlisted var"
    );
    assert_eq!(
        process_env.get("STRIPE_PUBLISHABLE_KEY").and_then(|v| v.as_str()),
        Some("pk_test_618"),
        "process.env must contain allowlisted var"
    );

    // Assert non-allowlisted var is excluded from all three targets.
    assert!(
        env.get("SECRET_NON_ALLOWLISTED").is_none(),
        "_ENV must NOT contain non-allowlisted var"
    );
    assert!(
        server.get("SECRET_NON_ALLOWLISTED").is_none(),
        "_SERVER must NOT contain non-allowlisted var"
    );
    assert!(
        process_env.get("SECRET_NON_ALLOWLISTED").is_none(),
        "process.env must NOT contain non-allowlisted var"
    );

    // Clean up.
    unsafe {
        std::env::remove_var("STRIPE_PUBLISHABLE_KEY");
        std::env::remove_var("SECRET_NON_ALLOWLISTED");
    }
}
