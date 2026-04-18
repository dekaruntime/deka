//! Built-in REST API for every shop — handled at the platform level.
//!
//! When a request hits `/api/*`, the deka platform skips the V8 isolate
//! entirely, queries Neo4j directly, and returns JSON. This provides a
//! stable, fast API surface for mobile apps, signup readiness checks,
//! and third-party integrations.
//!
//! ## Routes
//!
//! **Public (no auth):**
//! - `GET /api/products` — all products for a shop
//! - `GET /api/products/:sku` — single product by SKU
//! - `GET /api/categories` — distinct product categories
//! - `GET /api/shop` — shop info (name, tagline, logo, primaryColor, mode)

use axum::response::Response;
use neo4rs::{Graph, query};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

/// Hard ceiling on any single API query.
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Shared Neo4j graph connection for the API layer.
static API_GRAPH: OnceCell<Graph> = OnceCell::const_new();

/// Get or create the shared Neo4j connection for API queries.
async fn get_graph() -> Result<&'static Graph, String> {
    API_GRAPH
        .get_or_try_init(|| async {
            let uri = std::env::var("DEKA_NEO4J_URI")
                .unwrap_or_else(|_| "bolt://localhost:7687".to_string());
            let user = std::env::var("DEKA_NEO4J_USER")
                .unwrap_or_else(|_| "neo4j".to_string());
            let password = std::env::var("DEKA_NEO4J_PASSWORD")
                .unwrap_or_default();
            let db = std::env::var("DEKA_NEO4J_DB")
                .unwrap_or_else(|_| "neo4j".to_string());

            let config = neo4rs::ConfigBuilder::default()
                .uri(&uri)
                .user(&user)
                .password(&password)
                .db(&*db)
                .build()
                .map_err(|e| format!("neo4j config error: {}", e))?;

            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                Graph::connect(config),
            )
            .await
            .map_err(|_| "neo4j connect timeout".to_string())?
            .map_err(|e| format!("neo4j connect error: {}", e))
        })
        .await
}

/// Handle an `/api/*` request. Returns `Some(Response)` if the path
/// matched a known API route, `None` if it should fall through (unknown
/// `/api/` sub-path returns 404, not fall-through).
pub async fn handle_api_request(
    path: &str,
    headers: &[(String, String)],
) -> Response<axum::body::Body> {
    // Resolve shop_id from Host header
    let shop_id = match pool::tenant::resolve_tenant_from_host(headers) {
        Some(id) if !id.is_empty() => id,
        _ => {
            return json_response(404, &json!({ "error": "shop not found" }));
        }
    };

    // Route the API path
    if path == "/api/products" {
        api_products(&shop_id).await
    } else if path == "/api/categories" {
        api_categories(&shop_id).await
    } else if path == "/api/shop" {
        api_shop(&shop_id).await
    } else if let Some(sku) = path.strip_prefix("/api/products/") {
        if sku.is_empty() || sku.contains('/') {
            json_response(404, &json!({ "error": "not found" }))
        } else {
            api_product_by_sku(&shop_id, sku).await
        }
    } else {
        json_response(404, &json!({ "error": "not found" }))
    }
}

/// `GET /api/products` — all products for a shop.
async fn api_products(shop_id: &str) -> Response<axum::body::Body> {
    let graph = match get_graph().await {
        Ok(g) => g,
        Err(e) => return json_response(503, &json!({ "error": e })),
    };

    let cypher = "MATCH (s:Shop {id: $shopId})-[:HAS]->(p:Product) \
                  RETURN p.sku AS sku, p.name AS name, p.price AS price, \
                         p.description AS description, p.category AS category, \
                         p.imageUrl AS imageUrl";

    match run_query(graph, cypher, &[("shopId", json!(shop_id))], &PRODUCT_COLUMNS).await {
        Ok(rows) => json_response(200, &Value::Array(rows)),
        Err(e) => json_response(500, &json!({ "error": e })),
    }
}

/// `GET /api/products/:sku` — single product by SKU.
async fn api_product_by_sku(shop_id: &str, sku: &str) -> Response<axum::body::Body> {
    let graph = match get_graph().await {
        Ok(g) => g,
        Err(e) => return json_response(503, &json!({ "error": e })),
    };

    let cypher = "MATCH (s:Shop {id: $shopId})-[:HAS]->(p:Product {sku: $sku}) \
                  RETURN p.sku AS sku, p.name AS name, p.price AS price, \
                         p.description AS description, p.category AS category, \
                         p.imageUrl AS imageUrl";

    match run_query(
        graph,
        cypher,
        &[("shopId", json!(shop_id)), ("sku", json!(sku))],
        &PRODUCT_COLUMNS,
    )
    .await
    {
        Ok(rows) if rows.is_empty() => {
            json_response(404, &json!({ "error": "product not found" }))
        }
        Ok(mut rows) => json_response(200, &rows.remove(0)),
        Err(e) => json_response(500, &json!({ "error": e })),
    }
}

/// `GET /api/categories` — distinct product categories for a shop.
async fn api_categories(shop_id: &str) -> Response<axum::body::Body> {
    let graph = match get_graph().await {
        Ok(g) => g,
        Err(e) => return json_response(503, &json!({ "error": e })),
    };

    let cypher = "MATCH (s:Shop {id: $shopId})-[:HAS]->(p:Product) \
                  RETURN DISTINCT p.category AS category";

    match run_query(graph, cypher, &[("shopId", json!(shop_id))], &["category"]).await {
        Ok(rows) => {
            // Flatten to a simple array of strings
            let categories: Vec<Value> = rows
                .into_iter()
                .filter_map(|row| row.get("category").cloned())
                .filter(|v| !v.is_null())
                .collect();
            json_response(200, &Value::Array(categories))
        }
        Err(e) => json_response(500, &json!({ "error": e })),
    }
}

/// `GET /api/shop` — shop info.
async fn api_shop(shop_id: &str) -> Response<axum::body::Body> {
    let graph = match get_graph().await {
        Ok(g) => g,
        Err(e) => return json_response(503, &json!({ "error": e })),
    };

    let cypher = "MATCH (s:Shop {id: $shopId}) \
                  RETURN s.id AS id, s.name AS name, s.tagline AS tagline, \
                         s.logoUrl AS logoUrl, s.primaryColor AS primaryColor, \
                         s.mode AS mode";

    match run_query(
        graph,
        cypher,
        &[("shopId", json!(shop_id))],
        &["id", "name", "tagline", "logoUrl", "primaryColor", "mode"],
    )
    .await
    {
        Ok(rows) if rows.is_empty() => {
            json_response(404, &json!({ "error": "shop not found" }))
        }
        Ok(mut rows) => json_response(200, &rows.remove(0)),
        Err(e) => json_response(500, &json!({ "error": e })),
    }
}

/// Column names for product queries.
const PRODUCT_COLUMNS: [&str; 6] = ["sku", "name", "price", "description", "category", "imageUrl"];

/// Execute a Cypher query and return rows as JSON objects.
async fn run_query(
    graph: &Graph,
    cypher: &str,
    params: &[(&str, Value)],
    columns: &[&str],
) -> Result<Vec<Value>, String> {
    let mut q = query(cypher);
    for (key, val) in params {
        q = match val {
            Value::String(s) => q.param(key, s.as_str()),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    q.param(key, i)
                } else if let Some(f) = n.as_f64() {
                    q.param(key, f)
                } else {
                    q.param(key, n.to_string())
                }
            }
            _ => q.param(key, val.to_string()),
        };
    }

    let mut result = tokio::time::timeout(QUERY_TIMEOUT, graph.execute(q))
        .await
        .map_err(|_| format!("neo4j query timeout after {}s", QUERY_TIMEOUT.as_secs()))?
        .map_err(|e| format!("neo4j query error: {}", e))?;

    let mut rows = Vec::new();
    while let Some(row) = result
        .next()
        .await
        .map_err(|e| format!("neo4j row error: {}", e))?
    {
        let mut obj = serde_json::Map::new();
        for col in columns {
            let val = row_to_json(&row, col);
            obj.insert(col.to_string(), val);
        }
        rows.push(Value::Object(obj));
    }
    Ok(rows)
}

/// Extract a row column value to JSON — mirrors the logic in modules_php/neo4j.rs.
fn row_to_json(row: &neo4rs::Row, key: &str) -> Value {
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
    Value::Null
}

/// Build a JSON response with the given status code and body.
fn json_response(status: u16, body: &Value) -> Response<axum::body::Body> {
    let json_bytes = serde_json::to_vec(body).unwrap_or_default();
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("cache-control", "no-cache")
        .header("access-control-allow-origin", "*")
        .body(axum::body::Body::from(json_bytes))
        .unwrap()
}
