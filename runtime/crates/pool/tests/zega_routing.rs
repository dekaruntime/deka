use pool::{ExecutionMode, HandlerKey, IsolatePool, PoolConfig, RequestData, RequestParts};
use serde_json::{Value, json};
use std::sync::Arc;

fn mark_backend(root: &std::path::Path, shop_id: &str, backend: &str) {
    std::fs::create_dir_all(root.join(shop_id)).unwrap();
    std::fs::write(root.join(shop_id).join("backend"), backend).unwrap();
}

async fn execute_shop(pool: &IsolatePool, shop_id: &str) -> Value {
    let response = pool
        .execute(
            HandlerKey::new(format!("zega-routing-{shop_id}")),
            RequestData {
                handler_code: handler_code(),
                handler_entry: None,
                request_value: Value::Null,
                request_parts: Some(RequestParts {
                    url: "http://localhost/zega-routing".to_string(),
                    method: "GET".to_string(),
                    headers: vec![("host".to_string(), format!("{shop_id}.tana.gg"))],
                    body: None,
                }),
                mode: ExecutionMode::Request,
            },
        )
        .await
        .expect("execute isolate request");

    assert!(response.success, "isolate failed: {:?}", response.error);
    let body = response
        .result
        .and_then(|result| result.get("body").cloned())
        .and_then(|body| body.as_str().map(str::to_owned))
        .expect("response body");
    serde_json::from_str(&body).expect("JSON response body")
}

fn handler_code() -> String {
    r#"
function run_cql(handle, cqlValue) {
  if (!cqlValue || cqlValue.__type !== 'cql') {
    return { ok: false, error: 'run_cql expects a cql value', rows: [] };
  }
  return globalThis.__bridge('neo4j', 'query', {
    handle,
    cypher: cqlValue.query,
    params: cqlValue.params
  });
}

const app = {
  fetch() {
    const created = globalThis.__bridge('neo4j', 'execute', {
      handle: 0,
      cypher: 'CREATE (p:Product {name: $name})',
      params: { name: 'Hat' }
    });
    const cql = {
      __type: 'cql',
      query: 'MATCH (p:Product {name: $name}) RETURN p.name AS name',
      params: { name: 'Hat' }
    };
    const routed = run_cql(0, cql);
    const fallback = globalThis.__bridge('neo4j', 'routing_probe', {});
    return {
      status: 200,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ created, routed, fallback })
    };
  }
};
"#
    .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cql_envelope_routes_through_runtime_bridge_by_shop_backend() {
    let root = tempfile::tempdir().unwrap();
    mark_backend(root.path(), "shop_zega", "zega\n");
    mark_backend(root.path(), "shop_neo4j", "neo4j\n");
    unsafe {
        std::env::set_var("DEKA_TENANTS_DIR", root.path());
    }

    let pool = IsolatePool::new(
        PoolConfig {
            num_workers: 1,
            max_isolates_per_worker: 4,
            enable_code_cache: false,
            ..PoolConfig::default()
        },
        Arc::new(platform_server::extensions_for_php_server),
    );

    let zega = execute_shop(&pool, "shop_zega").await;
    assert_eq!(zega["created"], json!({ "ok": true }));
    assert_eq!(
        zega["routed"],
        json!({ "ok": true, "rows": [{ "name": "Hat" }], "count": 1 })
    );
    assert_eq!(
        zega["fallback"],
        json!({ "ok": false, "error": "unknown Zega CQL action 'routing_probe'" })
    );

    for shop_id in ["shop_neo4j", "shop_unmarked"] {
        let result = execute_shop(&pool, shop_id).await;
        assert_eq!(
            result["fallback"],
            json!({ "ok": false, "error": "unknown neo4j action 'routing_probe'" }),
            "{shop_id} should fall through to the Neo4j dispatcher"
        );
        assert_eq!(
            result["routed"]["ok"], false,
            "{shop_id} should not execute the CQL envelope in embedded Zega"
        );
    }

    std::mem::forget(pool);
}
