//! Neo4j bridge module — handles `bridge('neo4j', action, payload)` calls from PHPX.
//!
//! Uses neo4rs async driver with a thread-local tokio runtime for synchronous bridge calls.
//! Connection pooling is managed by neo4rs internally.

use neo4rs::{Graph, query};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;

thread_local! {
    static RT: RefCell<Option<Runtime>> = RefCell::new(None);
    static CONNECTIONS: RefCell<HashMap<u64, Arc<Graph>>> = RefCell::new(HashMap::new());
    static NEXT_HANDLE: RefCell<u64> = const { RefCell::new(1) };
}

/// Run an async operation, handling the case where we may already be inside a tokio runtime.
fn block_on_async<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    // If we're already in a tokio runtime (e.g., Deka's isolate pool),
    // spawn on a separate thread to avoid "cannot start runtime within runtime".
    if tokio::runtime::Handle::try_current().is_ok() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = Runtime::new().expect("failed to create tokio runtime");
            let result = rt.block_on(f);
            let _ = tx.send(result);
        });
        rx.recv().expect("neo4j worker thread panicked")
    } else {
        // Not inside a runtime — create one and block directly
        RT.with(|cell: &RefCell<Option<Runtime>>| {
            let mut rt = cell.borrow_mut();
            if rt.is_none() {
                *rt = Some(Runtime::new().expect("failed to create tokio runtime"));
            }
            rt.as_ref().unwrap().block_on(f)
        })
    }
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
    // Config priority: explicit args > env vars > defaults
    let uri = args
        .get("uri")
        .or_else(|| args.get("url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("DEKA_NEO4J_URI").unwrap_or_else(|_| "bolt://localhost:7687".to_string()));
    let user = args
        .get("user")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("DEKA_NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string()));
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

    let result = block_on_async(async move {
        let config = neo4rs::ConfigBuilder::default()
            .uri(&uri)
            .user(&user)
            .password(&password)
            .db(&*db)
            .build()
            .map_err(|e| format!("{}", e))?;
        Graph::connect(config)
            .await
            .map_err(|e| format!("{}", e))
    });

    match result {
        Ok(graph) => {
            let handle = NEXT_HANDLE.with(|h: &RefCell<u64>| {
                let mut h = h.borrow_mut();
                let id = *h;
                *h += 1;
                id
            });
            CONNECTIONS.with(|c: &RefCell<HashMap<u64, Arc<Graph>>>| {
                c.borrow_mut().insert(handle, Arc::new(graph));
            });
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
        cols.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
    } else {
        Vec::new()
    };

    let graph = CONNECTIONS.with(|c: &RefCell<HashMap<u64, Arc<Graph>>>| c.borrow().get(&handle).cloned());
    let graph = match graph {
        Some(g) => g,
        None => return json!({ "ok": false, "error": format!("invalid connection handle {}", handle) }),
    };

    let result = block_on_async(async move {
        let mut q = query(&cypher);

        if let Some(obj) = params.as_object() {
            for (key, val) in obj {
                q = bind_param(q, key, val);
            }
        }

        let mut result = graph.execute(q).await.map_err(|e| format!("{}", e))?;
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

    let graph = CONNECTIONS.with(|c: &RefCell<HashMap<u64, Arc<Graph>>>| c.borrow().get(&handle).cloned());
    let graph = match graph {
        Some(g) => g,
        None => return json!({ "ok": false, "error": format!("invalid connection handle {}", handle) }),
    };

    let result = block_on_async(async move {
        let mut q = query(&cypher);
        if let Some(obj) = params.as_object() {
            for (key, val) in obj {
                q = bind_param(q, key, val);
            }
        }
        graph.run(q).await.map_err(|e| format!("{}", e))
    });

    match result {
        Ok(_) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

fn neo4j_close(args: &Value) -> Value {
    let handle = args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0);
    let removed = CONNECTIONS.with(|c: &RefCell<HashMap<u64, Arc<Graph>>>| c.borrow_mut().remove(&handle).is_some());
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
            let strings: Vec<String> = arr.iter().map(|v| match v {
                Value::String(s) => s.clone(),
                _ => v.to_string(),
            }).collect();
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
    fn connect_to_local_neo4j() {
        let result = neo4j_connect(&json!({
            "uri": "bolt://localhost:7688",
            "user": "neo4j",
            "password": "deka_dev_password"
        }));
        assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true),
            "failed to connect: {:?}", result);
        let handle = result.get("handle").and_then(|v| v.as_u64()).unwrap();
        assert!(handle > 0);

        // Clean up
        neo4j_close(&json!({ "handle": handle }));
    }

    #[test]
    fn query_returns_results() {
        let conn = neo4j_connect(&json!({
            "uri": "bolt://localhost:7688",
            "user": "neo4j",
            "password": "deka_dev_password"
        }));
        let handle = conn.get("handle").and_then(|v| v.as_u64()).unwrap();

        // Create a test node
        let exec_result = neo4j_execute(&json!({
            "handle": handle,
            "cypher": "CREATE (n:DekaTest {name: $name}) RETURN n",
            "params": { "name": "test_product" }
        }));
        assert_eq!(exec_result.get("ok").and_then(|v| v.as_bool()), Some(true),
            "execute failed: {:?}", exec_result);

        // Query it back
        let query_result = neo4j_query(&json!({
            "handle": handle,
            "cypher": "MATCH (n:DekaTest {name: $name}) RETURN n.name AS name",
            "params": { "name": "test_product" },
            "columns": ["name"]
        }));
        assert_eq!(query_result.get("ok").and_then(|v| v.as_bool()), Some(true),
            "query failed: {:?}", query_result);
        let rows = query_result.get("rows").and_then(|v| v.as_array()).unwrap();
        assert!(!rows.is_empty());
        assert_eq!(rows[0].get("name").and_then(|v| v.as_str()), Some("test_product"));

        // Clean up test data
        neo4j_execute(&json!({
            "handle": handle,
            "cypher": "MATCH (n:DekaTest) DELETE n"
        }));
        neo4j_close(&json!({ "handle": handle }));
    }
}
