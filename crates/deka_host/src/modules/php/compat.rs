use super::*;
use crate::modules::{http, neo4j, redis_mod};

#[op2]
#[serde]
pub(super) fn op_redis_call(
    #[string] action: String,
    #[serde] args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    Ok(redis_mod::redis_call(&action, &args))
}

#[op2]
#[serde]
pub(super) fn op_neo4j_call(
    #[string] action: String,
    #[serde] args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    Ok(neo4j::neo4j_call(&action, &args))
}

/// @deka/http — outbound HTTP/1.1 + HTTP/2, streaming, cookie jars,
/// WebSocket client. See `crates/deka_host/src/modules/http.rs` for
/// the full action list and the DoD in issue #128.
#[op2]
#[serde]
pub(super) fn op_deka_http_call(
    #[string] action: String,
    #[serde] args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    Ok(http::http_call(&action, &args))
}

/// Introspect the shard layout for an `account_id`.
///
/// Empty `account_id` returns the local "self" shard if one is
/// configured, otherwise shard 0. Used by admin tools, debug
/// logging, and the `shard_for()` PHPX helper — the production path
/// (neo4j/redis connect) routes implicitly via `__account_id` in the
/// bridge payload, so this op is strictly for observability.
#[op2]
#[serde]
pub(super) fn op_shard_for(
    #[string] _account_id: String,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let neo4j_url = std::env::var("DEKA_NEO4J_URI")
        .unwrap_or_else(|_| "bolt://localhost:7687".to_string());
    let redis_url = std::env::var("DEKA_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());
    Ok(serde_json::json!({
        "ok": true,
        "index": 0,
        "name": "local",
        "neo4j_url": neo4j_url,
        "redis_url": redis_url,
        "owned": true,
        "self_name": "local",
    }))
}
