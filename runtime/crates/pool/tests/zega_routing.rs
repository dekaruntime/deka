use pool::{ExecutionMode, HandlerKey, IsolatePool, PoolConfig, RequestData, RequestParts};
use serde_json::{Value, json};
use std::path::Path;
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
    println!("{shop_id}: {body}");
    serde_json::from_str(&body).expect("JSON response body")
}

fn handler_code() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let bridge = std::fs::read_to_string(root.join("php_modules/core/bridge.phpx"))
        .expect("read core bridge PHPX");
    let neo4j = stdlib_phpx(
        &std::fs::read_to_string(root.join("php_modules/neo4j/index.phpx"))
            .expect("read neo4j PHPX"),
        "neo4j",
    );
    let redis = stdlib_phpx(
        &std::fs::read_to_string(root.join("php_modules/redis/index.phpx"))
            .expect("read redis PHPX"),
        "redis",
    );
    let fixture = include_str!("fixtures/zega_routing.phpx");
    let source = format!(
        "{}\n{}\n{}\n{}",
        standalone_phpx(&bridge),
        neo4j,
        redis,
        standalone_phpx(fixture)
    );
    let mut js = phpx_js::compile_phpx_source_to_js(
        &source,
        "php_modules/zega_routing_integration.phpx",
        phpx_js::SourceModuleMeta::empty(),
    )
    .expect("compile Zega routing PHPX fixture");
    js = js.replace("export const ", "const ");
    // The PHPX emitter currently resolves function-local CQL bindings globally.
    js = js.replace("run_cql(0, globalThis.product)", "run_cql(0, product)");
    js.push_str(
        r#"
const app = {
  fetch() {
    return {
      status: 200,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(zega_routing_response())
    };
  }
};
"#,
    );
    js
}

fn standalone_phpx(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| line.replacen("export function ", "function ", 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stdlib_phpx(source: &str, prefix: &str) -> String {
    let mut source = standalone_phpx(source);
    for name in [
        "connect",
        "query",
        "execute",
        "close",
        "get",
        "set",
        "del",
        "exists",
        "expire",
        "ttl",
        "incr",
        "decr",
        "hget",
        "hset",
        "hgetall",
        "hdel",
        "lpush",
        "rpush",
        "lpop",
        "rpop",
        "lrange",
        "llen",
        "ltrim",
        "sadd",
        "smembers",
        "srem",
        "sismember",
        "keys",
        "flush",
    ] {
        source = source.replace(
            &format!("function {name}("),
            &format!("function {prefix}_{name}("),
        );
    }
    if prefix == "neo4j" {
        source = source.replace("return query(", "return neo4j_query(");
    }
    source
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
    assert_eq!(zega["kv_set"], json!({ "ok": true }));
    assert_eq!(zega["kv_get"], json!({ "ok": true, "value": "sku-1" }));
    assert_eq!(
        zega["kv_expire"],
        json!({ "ok": false, "error": "Redis action 'expire' is not supported by Zega" })
    );
    assert_eq!(
        zega["cql_probe"],
        json!({ "ok": false, "error": "unknown Zega CQL action 'routing_probe'" })
    );
    assert_eq!(
        zega["kv_probe"],
        json!({ "ok": false, "error": "missing 'key'" })
    );

    for shop_id in ["shop_neo4j", "shop_unmarked"] {
        let result = execute_shop(&pool, shop_id).await;
        assert_eq!(
            result["cql_probe"],
            json!({ "ok": false, "error": "unknown neo4j action 'routing_probe'" }),
            "{shop_id} should fall through to the Neo4j dispatcher"
        );
        assert_eq!(
            result["kv_probe"],
            json!({ "ok": false, "error": "unknown redis action 'routing_probe'" }),
            "{shop_id} should fall through to the Redis dispatcher"
        );
        assert_eq!(
            result["routed"]["ok"], false,
            "{shop_id} should not execute the CQL envelope in embedded Zega"
        );
    }

    std::mem::forget(pool);
}
