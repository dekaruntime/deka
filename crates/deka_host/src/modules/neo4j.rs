//! Neo4j bridge module — handles `bridge('neo4j', action, payload)` calls from PHPX.
//!
//! Uses neo4rs async driver with a thread-local tokio runtime for synchronous bridge calls.
//! Connection pooling is managed by neo4rs internally.

use neo4rs::{Graph, query};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

/// Global connection store — shared across all threads, protected by mutex.
/// Connections live here so they stay on the neo4j runtime's context.
use std::sync::OnceLock;
static CONNECTIONS: OnceLock<Mutex<HashMap<u64, Arc<Graph>>>> = OnceLock::new();
static NEXT_HANDLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn connections() -> &'static Mutex<HashMap<u64, Arc<Graph>>> {
    CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Hard ceiling on how long a Bolt connect can block before the op fails.
/// Without this, a handler targeting an unreachable Bolt endpoint (e.g. a
/// tenant hardcoded to `bolt://localhost:7688` that's been migrated to a
/// shard whose docker stack only exposes 7687) will hang the request
/// forever — a startup hang that blocked an earlier migration.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Hard ceiling on a single Cypher query / execute.
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Dedicated multi-thread tokio runtime for neo4j async I/O.
/// Has its own worker threads so async I/O is driven independently of the calling thread.
static NEO4J_RT: OnceLock<tokio::runtime::Handle> = OnceLock::new();

fn neo4j_handle() -> &'static tokio::runtime::Handle {
    NEO4J_RT.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to create neo4j tokio runtime");
        let handle = rt.handle().clone();
        // Leak the runtime — it lives for the process lifetime
        std::mem::forget(rt);
        handle
    })
}

/// Run an async neo4j operation synchronously.
///
/// Spawns the future on the dedicated multi-thread runtime (which has its own worker
/// threads for I/O). The calling thread blocks on a sync channel. The key insight:
/// the neo4j runtime's worker threads drive the I/O independently, so blocking the
/// caller doesn't prevent progress.
fn block_on_async<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handle = neo4j_handle();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    handle.spawn(async move {
        let result = f.await;
        let _ = tx.send(result);
    });
    rx.recv().expect("neo4j async task failed")
}

/// Pick a Neo4j URL from the shard resolver for a connect() call that
/// omits an explicit URL.
///
/// Precedence:
///   1. If the payload carries an `__account_id`, route to that shop's
///      shard directly — this is the PHPX storefront hot path.
///   2. Otherwise, if the resolver is configured for a real cluster
///      (more than one shard, or a named shard that isn't the
///      `local` fallback), use shard 0 (the de-facto router shard).
///   3. Otherwise, honour `DEKA_NEO4J_URI` — this preserves the
///      single-machine dev default where docker-compose publishes
///      Neo4j on a non-standard port (e.g. 7688).
///   4. Finally fall back to `bolt://localhost:7687`.
fn shard_route_neo4j(_args: &Value) -> String {
    std::env::var("DEKA_NEO4J_URI").unwrap_or_else(|_| "bolt://localhost:7687".to_string())
}

/// Previously true iff this process was running inside a multi-shard
/// deployment. The shard subsystem has been archived, so tenant code
/// always uses explicit URLs or the process env defaults.
pub(crate) fn has_configured_cluster() -> bool {
    false
}

/// Heuristic: does `url` look like a single-machine dev default?
/// We treat bolt / neo4j / redis / http URLs pointing at `localhost`
/// or `127.0.0.1` as dev defaults worth replacing with the
/// shard-routed URL in a configured cluster. This covers tenant
/// handlers that still hardcode `bolt://localhost:7688` /
/// `redis://localhost:6380` instead of calling the no-arg
/// auto-routing form.
pub(crate) fn should_override_dev_default(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    // Cheap substring check — avoids pulling in the url crate for a
    // simple host match.
    let strip_scheme = lower
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(lower.as_str());
    // Host portion ends at `/`, `?`, or end-of-string.
    let host_port = strip_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or(strip_scheme);
    // IPv6 literals are wrapped in [] — `[::1]:7687`. Strip the brackets
    // before comparing.
    let host = if let Some(inner) = host_port
        .strip_prefix('[')
        .and_then(|s| s.split_once(']').map(|(h, _)| h))
    {
        inner
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

/// Tiny log helper — matches the `[category] message` format used by
/// `crates/runtime/src/platform.rs::stdio`.
fn stdio_log(message: &str) {
    eprintln!("[shard] {}", message);
}

/// Main dispatch function — called from the JS bridge router via op_neo4j_call.
pub fn neo4j_call(action: &str, args: &Value) -> Value {
    match action {
        "connect" => neo4j_connect(args),
        "query" => neo4j_query(args),
        "execute" => neo4j_execute(args),
        "close" => neo4j_close(args),
        _ => json!({ "ok": false, "error": format!("unknown neo4j action '{}'", action) }),
    }
}

fn neo4j_connect(args: &Value) -> Value {
    // Config priority: shard resolver (from __account_id in a configured cluster)
    // > explicit args > env vars > defaults.
    //
    // The JS bridge router stamps `__account_id` into the payload on
    // connect() calls. The resolver maps that to the neo4j URL of
    // the owning shard. We override an explicit URL with the
    // shard-routed URL when (a) the cluster is configured with more
    // than one shard, and (b) the explicit URL looks like the
    // single-machine dev default (`localhost` / `127.0.0.1`). This
    // is a safety net for PHPX handlers that still hardcode
    // `bolt://localhost:7688` instead of calling the no-arg form.
    // Without this override, migrated tenants on non-router shards
    // can hang forever connecting to a port their docker
    // stack doesn't expose.
    let explicit = args
        .get("uri")
        .or_else(|| args.get("url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let shard_routed = shard_route_neo4j(args);
    let uri = match explicit {
        Some(u) if should_override_dev_default(&u) && has_configured_cluster() => {
            stdio_log(&format!(
                "neo4j: overriding dev-default URL {} with shard-routed {} (account={})",
                u,
                shard_routed,
                args.get("__account_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<none>"),
            ));
            shard_routed
        }
        Some(u) => u,
        None => shard_routed,
    };
    let user = args
        .get("user")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::env::var("DEKA_NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string())
        });
    let password = args
        .get("password")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("DEKA_NEO4J_PASSWORD").unwrap_or_default());
    let db = args
        .get("db")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("DEKA_NEO4J_DB").unwrap_or_else(|_| "neo4j".to_string()));

    let uri_for_err = uri.clone();
    let result = block_on_async(async move {
        let config = neo4rs::ConfigBuilder::default()
            .uri(&uri)
            .user(&user)
            .password(&password)
            .db(&*db)
            .build()
            .map_err(|e| format!("{}", e))?;
        // Hard connect timeout — without this, a handler targeting an
        // unreachable Bolt endpoint will block the worker thread
        // indefinitely (see Phase 6 hang diagnosis).
        tokio::time::timeout(CONNECT_TIMEOUT, Graph::connect(config))
            .await
            .map_err(|_| {
                format!(
                    "neo4j connect timeout after {}s for {}",
                    CONNECT_TIMEOUT.as_secs(),
                    uri_for_err
                )
            })?
            .map_err(|e| format!("{}", e))
    });

    match result {
        Ok(graph) => {
            let handle = NEXT_HANDLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            connections()
                .lock()
                .unwrap()
                .insert(handle, Arc::new(graph));
            json!({ "ok": true, "handle": handle })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

fn neo4j_query(args: &Value) -> Value {
    let handle = args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0);
    let cypher = match args.get("cypher").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return json!({ "ok": false, "error": "missing 'cypher' field" }),
    };
    let params = args.get("params").cloned().unwrap_or(json!({}));
    let columns: Vec<String> = if let Some(cols) = args.get("columns").and_then(|v| v.as_array()) {
        cols.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    } else {
        Vec::new()
    };

    let graph = connections().lock().unwrap().get(&handle).cloned();
    let graph = match graph {
        Some(g) => g,
        None => {
            return json!({ "ok": false, "error": format!("invalid connection handle {}", handle) });
        }
    };

    let result = block_on_async(async move {
        let mut q = query(&cypher);

        if let Some(obj) = params.as_object() {
            for (key, val) in obj {
                q = bind_param(q, key, val);
            }
        }

        // Hard query timeout — mirrors the connect timeout. If the
        // server is unreachable mid-stream, this caps how long a
        // single handler can block waiting on Neo4j.
        let mut result = tokio::time::timeout(QUERY_TIMEOUT, graph.execute(q))
            .await
            .map_err(|_| format!("neo4j query timeout after {}s", QUERY_TIMEOUT.as_secs()))?
            .map_err(|e| format!("{}", e))?;
        let mut rows = Vec::new();

        while let Some(row) = result.next().await.map_err(|e| format!("{}", e))? {
            let mut row_obj = serde_json::Map::new();
            if columns.is_empty() {
                if let Ok(v) = row.get::<String>("") {
                    row_obj.insert("_".to_string(), json!(v));
                }
            } else {
                for key in &columns {
                    let val = row_to_json(&row, key);
                    row_obj.insert(key.clone(), val);
                }
            }
            rows.push(Value::Object(row_obj));
        }

        Ok::<Vec<Value>, String>(rows)
    });

    match result {
        Ok(rows) => json!({ "ok": true, "rows": rows, "count": rows.len() }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

fn neo4j_execute(args: &Value) -> Value {
    // Execute is like query but doesn't collect results — for CREATE, SET, DELETE
    let handle = args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0);
    let cypher = match args.get("cypher").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return json!({ "ok": false, "error": "missing 'cypher' field" }),
    };
    let params = args.get("params").cloned().unwrap_or(json!({}));

    let graph = connections().lock().unwrap().get(&handle).cloned();
    let graph = match graph {
        Some(g) => g,
        None => {
            return json!({ "ok": false, "error": format!("invalid connection handle {}", handle) });
        }
    };

    let result = block_on_async(async move {
        let mut q = query(&cypher);
        if let Some(obj) = params.as_object() {
            for (key, val) in obj {
                q = bind_param(q, key, val);
            }
        }

        // Same hard timeout as neo4j_query to cap worker blocking.
        tokio::time::timeout(QUERY_TIMEOUT, graph.run(q))
            .await
            .map_err(|_| format!("neo4j execute timeout after {}s", QUERY_TIMEOUT.as_secs()))?
            .map_err(|e| format!("{}", e))?;
        Ok::<(), String>(())
    });

    match result {
        Ok(_) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

fn neo4j_close(args: &Value) -> Value {
    let handle = args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0);
    let removed = connections().lock().unwrap().remove(&handle).is_some();
    if removed {
        json!({ "ok": true })
    } else {
        json!({ "ok": false, "error": format!("invalid connection handle {}", handle) })
    }
}

/// Bind a JSON value as a Cypher query parameter.
fn bind_param(q: neo4rs::Query, key: &str, val: &Value) -> neo4rs::Query {
    match val {
        Value::Null => q.param(key, Option::<String>::None),
        Value::Bool(b) => q.param(key, *b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.param(key, i)
            } else if let Some(f) = n.as_f64() {
                q.param(key, f)
            } else {
                q.param(key, n.to_string())
            }
        }
        Value::String(s) => q.param(key, s.as_str()),
        Value::Array(arr) => {
            // Convert to Vec<String> for simplicity — Neo4j supports typed lists
            // but JSON arrays are heterogeneous
            let strings: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    _ => v.to_string(),
                })
                .collect();
            q.param(key, strings)
        }
        Value::Object(_) => {
            // Pass as JSON string — Neo4j doesn't natively support map params in all contexts
            q.param(key, val.to_string())
        }
    }
}

/// Extract a row column value to JSON.
fn row_to_json(row: &neo4rs::Row, key: &str) -> Value {
    // Try each type in order of likelihood
    if let Ok(v) = row.get::<String>(key) {
        return Value::String(v);
    }
    if let Ok(v) = row.get::<i64>(key) {
        return json!(v);
    }
    if let Ok(v) = row.get::<f64>(key) {
        return json!(v);
    }
    if let Ok(v) = row.get::<bool>(key) {
        return json!(v);
    }
    // Node — extract properties
    if let Ok(node) = row.get::<neo4rs::Node>(key) {
        let mut obj = serde_json::Map::new();
        obj.insert("__type".to_string(), json!("node"));
        obj.insert("id".to_string(), json!(node.id()));
        obj.insert("labels".to_string(), json!(node.labels()));
        return Value::Object(obj);
    }
    // Relationship
    if let Ok(rel) = row.get::<neo4rs::Relation>(key) {
        let mut obj = serde_json::Map::new();
        obj.insert("__type".to_string(), json!("relationship"));
        obj.insert("id".to_string(), json!(rel.id()));
        obj.insert("start_node_id".to_string(), json!(rel.start_node_id()));
        obj.insert("end_node_id".to_string(), json!(rel.end_node_id()));
        obj.insert("typ".to_string(), json!(rel.typ()));
        return Value::Object(obj);
    }
    // Fallback — null
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_default_url_detection() {
        assert!(should_override_dev_default("bolt://localhost:7688"));
        assert!(should_override_dev_default("bolt://localhost:7687"));
        assert!(should_override_dev_default("bolt://127.0.0.1:7687"));
        assert!(should_override_dev_default("BOLT://LOCALHOST:7688"));
        assert!(should_override_dev_default("redis://localhost:6380"));
        assert!(should_override_dev_default("redis://localhost"));
        assert!(should_override_dev_default("bolt://127.0.0.1/db"));
    }

    #[test]
    fn connect_to_local_neo4j() {
        let result = neo4j_connect(&json!({
            "uri": "bolt://localhost:7688",
            "user": "neo4j",
            "password": ""
        }));
        assert_eq!(
            result.get("ok").and_then(|v| v.as_bool()),
            Some(true),
            "failed to connect: {:?}",
            result
        );
        let handle = result.get("handle").and_then(|v| v.as_u64()).unwrap();
        assert!(handle > 0);

        // Clean up
        neo4j_close(&json!({ "handle": handle }));
    }

    #[test]
    #[ignore = "requires local neo4j service; environmental test (see dekaruntime/deka#330)"]
    fn query_returns_results() {
        let conn = neo4j_connect(&json!({
            "uri": "bolt://localhost:7688",
            "user": "neo4j",
            "password": ""
        }));
        let handle = conn.get("handle").and_then(|v| v.as_u64()).unwrap();

        // Create a test node
        let exec_result = neo4j_execute(&json!({
            "handle": handle,
            "cypher": "CREATE (n:DekaTest {name: $name}) RETURN n",
            "params": { "name": "test_product" }
        }));
        assert_eq!(
            exec_result.get("ok").and_then(|v| v.as_bool()),
            Some(true),
            "execute failed: {:?}",
            exec_result
        );

        // Query it back
        let query_result = neo4j_query(&json!({
            "handle": handle,
            "cypher": "MATCH (n:DekaTest {name: $name}) RETURN n.name AS name",
            "params": { "name": "test_product" },
            "columns": ["name"]
        }));
        assert_eq!(
            query_result.get("ok").and_then(|v| v.as_bool()),
            Some(true),
            "query failed: {:?}",
            query_result
        );
        let rows = query_result.get("rows").and_then(|v| v.as_array()).unwrap();
        assert!(!rows.is_empty());
        assert_eq!(
            rows[0].get("name").and_then(|v| v.as_str()),
            Some("test_product")
        );

        // Clean up test data
        neo4j_execute(&json!({
            "handle": handle,
            "cypher": "MATCH (n:DekaTest) DELETE n"
        }));
        neo4j_close(&json!({ "handle": handle }));
    }
}
