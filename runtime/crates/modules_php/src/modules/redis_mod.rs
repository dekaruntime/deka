//! Redis bridge module — handles `bridge('redis', action, payload)` calls from PHPX.
//!
//! Uses the sync redis crate. Connection handles are thread-local.

use redis::{Client, Commands, Connection};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CONNECTIONS: RefCell<HashMap<u64, Connection>> = RefCell::new(HashMap::new());
    static NEXT_HANDLE: RefCell<u64> = const { RefCell::new(1) };
}

/// Pick a Redis URL from the shard resolver for a connect() call that
/// omits an explicit URL. See `shard_route_neo4j` for the rationale —
/// this mirrors that logic exactly.
fn shard_route_redis(args: &Value) -> String {
    let account_id = args
        .get("__account_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let resolver = deka_shard::global();

    if !account_id.is_empty() {
        if let Some(info) = resolver.resolve(account_id) {
            return info.redis.clone();
        }
    }

    let is_configured_cluster = resolver.shard_count() > 1
        || resolver
            .shards()
            .first()
            .map(|s| s.name != "local")
            .unwrap_or(false);

    if is_configured_cluster {
        if let Some(info) = resolver.shards().first() {
            return info.redis.clone();
        }
    }

    std::env::var("DEKA_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string())
}

/// Main dispatch function — called from the JS bridge router via op_redis_call.
pub fn redis_call(action: &str, args: &Value) -> Value {
    match action {
        "connect" => redis_connect(args),
        "get" => redis_get(args),
        "set" => redis_set(args),
        "del" => redis_del(args),
        "exists" => redis_exists(args),
        "expire" => redis_expire(args),
        "ttl" => redis_ttl(args),
        "incr" => redis_incr(args),
        "decr" => redis_decr(args),
        "hget" => redis_hget(args),
        "hset" => redis_hset(args),
        "hgetall" => redis_hgetall(args),
        "hdel" => redis_hdel(args),
        "lpush" => redis_lpush(args),
        "rpush" => redis_rpush(args),
        "lpop" => redis_lpop(args),
        "rpop" => redis_rpop(args),
        "lrange" => redis_lrange(args),
        "llen" => redis_llen(args),
        "sadd" => redis_sadd(args),
        "smembers" => redis_smembers(args),
        "srem" => redis_srem(args),
        "sismember" => redis_sismember(args),
        "keys" => redis_keys(args),
        "flush" => redis_flush(args),
        "close" => redis_close(args),
        _ => json!({ "ok": false, "error": format!("unknown redis action '{}'", action) }),
    }
}

fn get_handle(args: &Value) -> u64 {
    args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0)
}

fn with_conn<F>(handle: u64, f: F) -> Value
where
    F: FnOnce(&mut Connection) -> Value,
{
    CONNECTIONS.with(|c: &RefCell<HashMap<u64, Connection>>| {
        let mut conns = c.borrow_mut();
        match conns.get_mut(&handle) {
            Some(conn) => f(conn),
            None => json!({ "ok": false, "error": format!("invalid redis handle {}", handle) }),
        }
    })
}

fn redis_connect(args: &Value) -> Value {
    // Config priority: shard resolver (when the cluster is configured and
    // the explicit URL looks like a dev default) > explicit args > env
    // vars > default. See `neo4j_connect` for the full reasoning — this
    // is the same safety net for tenant handlers that hardcode
    // `redis://localhost:6380`.
    let explicit = args
        .get("url")
        .or_else(|| args.get("uri"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let shard_routed = shard_route_redis(args);
    let url_owned = match explicit {
        Some(u)
            if super::neo4j::should_override_dev_default(&u)
                && super::neo4j::has_configured_cluster() =>
        {
            eprintln!(
                "[shard] redis: overriding dev-default URL {} with shard-routed {} (account={})",
                u,
                shard_routed,
                args.get("__account_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<none>"),
            );
            shard_routed
        }
        Some(u) => u,
        None => shard_routed,
    };
    let url = url_owned.as_str();

    let client = match Client::open(url) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": format!("{}", e) }),
    };

    // Bounded connect timeout: without this a handler pointing at an
    // unreachable Redis (e.g. `redis://localhost:6380` on a shard that
    // only exposes 6379) blocks the worker thread forever. Matches the
    // 5s cap on the neo4j side.
    let conn = match client.get_connection_with_timeout(std::time::Duration::from_secs(5)) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": format!("{}", e) }),
    };

    let handle = NEXT_HANDLE.with(|h: &RefCell<u64>| {
        let mut h = h.borrow_mut();
        let id = *h;
        *h += 1;
        id
    });
    CONNECTIONS.with(|c: &RefCell<HashMap<u64, Connection>>| {
        c.borrow_mut().insert(handle, conn);
    });
    json!({ "ok": true, "handle": handle })
}

// ─── String commands ───────────────────────────────────────────

fn redis_get(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| match conn.get::<_, Option<String>>(&key) {
        Ok(Some(val)) => json!({ "ok": true, "value": val }),
        Ok(None) => json!({ "ok": true, "value": null }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_set(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let value = match args.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => return json!({ "ok": false, "error": "missing 'value'" }),
    };
    let ttl = args.get("ttl").and_then(|v| v.as_u64());

    with_conn(handle, |conn| {
        let result: Result<(), _> = if let Some(seconds) = ttl {
            conn.set_ex(&key, &value, seconds)
        } else {
            conn.set(&key, &value)
        };
        match result {
            Ok(_) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_del(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| match conn.del::<_, i64>(&key) {
        Ok(count) => json!({ "ok": true, "deleted": count }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_exists(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| match conn.exists::<_, bool>(&key) {
        Ok(exists) => json!({ "ok": true, "exists": exists }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_expire(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let seconds = args.get("seconds").and_then(|v| v.as_i64()).unwrap_or(0);
    with_conn(handle, |conn| match conn.expire::<_, bool>(&key, seconds) {
        Ok(set) => json!({ "ok": true, "set": set }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_ttl(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| match conn.ttl::<_, i64>(&key) {
        Ok(ttl) => json!({ "ok": true, "ttl": ttl }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_incr(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let by = args.get("by").and_then(|v| v.as_i64()).unwrap_or(1);
    with_conn(handle, |conn| match conn.incr::<_, _, i64>(&key, by) {
        Ok(val) => json!({ "ok": true, "value": val }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_decr(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let by = args.get("by").and_then(|v| v.as_i64()).unwrap_or(1);
    with_conn(handle, |conn| match conn.decr::<_, _, i64>(&key, by) {
        Ok(val) => json!({ "ok": true, "value": val }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

// ─── Hash commands ─────────────────────────────────────────────

fn redis_hget(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let field = match args.get("field").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => return json!({ "ok": false, "error": "missing 'field'" }),
    };
    with_conn(handle, |conn| {
        match conn.hget::<_, _, Option<String>>(&key, &field) {
            Ok(Some(val)) => json!({ "ok": true, "value": val }),
            Ok(None) => json!({ "ok": true, "value": null }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_hset(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let field = match args.get("field").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => return json!({ "ok": false, "error": "missing 'field'" }),
    };
    let value = match args.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => return json!({ "ok": false, "error": "missing 'value'" }),
    };
    with_conn(handle, |conn| {
        match conn.hset::<_, _, _, bool>(&key, &field, &value) {
            Ok(_) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_hgetall(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| {
        match conn.hgetall::<_, HashMap<String, String>>(&key) {
            Ok(map) => {
                let obj: serde_json::Map<String, Value> = map
                    .into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect();
                json!({ "ok": true, "value": Value::Object(obj) })
            }
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_hdel(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let field = match args.get("field").and_then(|v| v.as_str()) {
        Some(f) => f.to_string(),
        None => return json!({ "ok": false, "error": "missing 'field'" }),
    };
    with_conn(handle, |conn| match conn.hdel::<_, _, bool>(&key, &field) {
        Ok(_) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

// ─── List commands ─────────────────────────────────────────────

fn redis_lpush(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let value = match args.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => return json!({ "ok": false, "error": "missing 'value'" }),
    };
    with_conn(handle, |conn| match conn.lpush::<_, _, i64>(&key, &value) {
        Ok(len) => json!({ "ok": true, "length": len }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_rpush(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let value = match args.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => return json!({ "ok": false, "error": "missing 'value'" }),
    };
    with_conn(handle, |conn| match conn.rpush::<_, _, i64>(&key, &value) {
        Ok(len) => json!({ "ok": true, "length": len }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_lpop(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| {
        match conn.lpop::<_, Option<String>>(&key, None) {
            Ok(Some(val)) => json!({ "ok": true, "value": val }),
            Ok(None) => json!({ "ok": true, "value": null }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_rpop(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| {
        match conn.rpop::<_, Option<String>>(&key, None) {
            Ok(Some(val)) => json!({ "ok": true, "value": val }),
            Ok(None) => json!({ "ok": true, "value": null }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_lrange(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let start = args.get("start").and_then(|v| v.as_i64()).unwrap_or(0) as isize;
    let stop = args.get("stop").and_then(|v| v.as_i64()).unwrap_or(-1) as isize;
    with_conn(handle, |conn| {
        match conn.lrange::<_, Vec<String>>(&key, start, stop) {
            Ok(vals) => json!({ "ok": true, "values": vals }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_llen(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| match conn.llen::<_, i64>(&key) {
        Ok(len) => json!({ "ok": true, "length": len }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

// ─── Set commands ──────────────────────────────────────────────

fn redis_sadd(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let member = match args.get("member").or_else(|| args.get("value")) {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => return json!({ "ok": false, "error": "missing 'member'" }),
    };
    with_conn(handle, |conn| {
        match conn.sadd::<_, _, bool>(&key, &member) {
            Ok(added) => json!({ "ok": true, "added": added }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_smembers(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    with_conn(handle, |conn| match conn.smembers::<_, Vec<String>>(&key) {
        Ok(members) => json!({ "ok": true, "members": members }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_srem(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let member = match args.get("member").or_else(|| args.get("value")) {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => return json!({ "ok": false, "error": "missing 'member'" }),
    };
    with_conn(handle, |conn| {
        match conn.srem::<_, _, bool>(&key, &member) {
            Ok(removed) => json!({ "ok": true, "removed": removed }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_sismember(args: &Value) -> Value {
    let handle = get_handle(args);
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return json!({ "ok": false, "error": "missing 'key'" }),
    };
    let member = match args.get("member").or_else(|| args.get("value")) {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => return json!({ "ok": false, "error": "missing 'member'" }),
    };
    with_conn(handle, |conn| {
        match conn.sismember::<_, _, bool>(&key, &member) {
            Ok(is_member) => json!({ "ok": true, "is_member": is_member }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

// ─── Utility commands ──────────────────────────────────────────

fn redis_keys(args: &Value) -> Value {
    let handle = get_handle(args);
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");
    with_conn(handle, |conn| match conn.keys::<_, Vec<String>>(pattern) {
        Ok(keys) => json!({ "ok": true, "keys": keys }),
        Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
    })
}

fn redis_flush(args: &Value) -> Value {
    let handle = get_handle(args);
    with_conn(handle, |conn| {
        match redis::cmd("FLUSHDB").query::<String>(conn) {
            Ok(_) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": format!("{}", e) }),
        }
    })
}

fn redis_close(args: &Value) -> Value {
    let handle = get_handle(args);
    let removed = CONNECTIONS
        .with(|c: &RefCell<HashMap<u64, Connection>>| c.borrow_mut().remove(&handle).is_some());
    if removed {
        json!({ "ok": true })
    } else {
        json!({ "ok": false, "error": format!("invalid redis handle {}", handle) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_and_set_get() {
        let conn = redis_connect(&json!({ "url": "redis://localhost:6380" }));
        assert_eq!(
            conn.get("ok").and_then(|v| v.as_bool()),
            Some(true),
            "connect failed: {:?}",
            conn
        );
        let handle = conn.get("handle").and_then(|v| v.as_u64()).unwrap();

        // SET
        let set_result = redis_set(&json!({
            "handle": handle, "key": "deka:test:hello", "value": "world"
        }));
        assert_eq!(set_result.get("ok").and_then(|v| v.as_bool()), Some(true));

        // GET
        let get_result = redis_get(&json!({ "handle": handle, "key": "deka:test:hello" }));
        assert_eq!(get_result.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            get_result.get("value").and_then(|v| v.as_str()),
            Some("world")
        );

        // SET with TTL
        let set_ttl = redis_set(&json!({
            "handle": handle, "key": "deka:test:ttl", "value": "expires", "ttl": 60
        }));
        assert_eq!(set_ttl.get("ok").and_then(|v| v.as_bool()), Some(true));
        let ttl = redis_ttl(&json!({ "handle": handle, "key": "deka:test:ttl" }));
        assert!(ttl.get("ttl").and_then(|v| v.as_i64()).unwrap() > 0);

        // DEL
        redis_del(&json!({ "handle": handle, "key": "deka:test:hello" }));
        redis_del(&json!({ "handle": handle, "key": "deka:test:ttl" }));
        redis_close(&json!({ "handle": handle }));
    }

    #[test]
    fn hash_operations() {
        let conn = redis_connect(&json!({ "url": "redis://localhost:6380" }));
        let handle = conn.get("handle").and_then(|v| v.as_u64()).unwrap();

        redis_hset(
            &json!({ "handle": handle, "key": "deka:test:hash", "field": "name", "value": "alice" }),
        );
        redis_hset(
            &json!({ "handle": handle, "key": "deka:test:hash", "field": "age", "value": "30" }),
        );

        let get =
            redis_hget(&json!({ "handle": handle, "key": "deka:test:hash", "field": "name" }));
        assert_eq!(get.get("value").and_then(|v| v.as_str()), Some("alice"));

        let all = redis_hgetall(&json!({ "handle": handle, "key": "deka:test:hash" }));
        let val = all.get("value").and_then(|v| v.as_object()).unwrap();
        assert_eq!(val.get("name").and_then(|v| v.as_str()), Some("alice"));
        assert_eq!(val.get("age").and_then(|v| v.as_str()), Some("30"));

        redis_del(&json!({ "handle": handle, "key": "deka:test:hash" }));
        redis_close(&json!({ "handle": handle }));
    }

    #[test]
    fn list_operations() {
        let conn = redis_connect(&json!({ "url": "redis://localhost:6380" }));
        let handle = conn.get("handle").and_then(|v| v.as_u64()).unwrap();

        redis_rpush(&json!({ "handle": handle, "key": "deka:test:list", "value": "a" }));
        redis_rpush(&json!({ "handle": handle, "key": "deka:test:list", "value": "b" }));
        redis_rpush(&json!({ "handle": handle, "key": "deka:test:list", "value": "c" }));

        let len = redis_llen(&json!({ "handle": handle, "key": "deka:test:list" }));
        assert_eq!(len.get("length").and_then(|v| v.as_i64()), Some(3));

        let range = redis_lrange(
            &json!({ "handle": handle, "key": "deka:test:list", "start": 0, "stop": -1 }),
        );
        let vals = range.get("values").and_then(|v| v.as_array()).unwrap();
        assert_eq!(vals.len(), 3);

        let popped = redis_lpop(&json!({ "handle": handle, "key": "deka:test:list" }));
        assert_eq!(popped.get("value").and_then(|v| v.as_str()), Some("a"));

        redis_del(&json!({ "handle": handle, "key": "deka:test:list" }));
        redis_close(&json!({ "handle": handle }));
    }
}
