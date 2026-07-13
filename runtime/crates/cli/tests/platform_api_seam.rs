extern crate std as core;

use reqwest::Client;
use serde_json::{Value, json};
use std::fs;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

struct PlatformProcess {
    child: Child,
    _root: TempDir,
}

impl Drop for PlatformProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn platform_api_keeps_product_reads_scoped_to_host_tenant_despite_bearer_token() {
    let Some(neo4j_password) = std::env::var("NEO4J_PASSWORD")
        .ok()
        .or_else(|| std::env::var("DEKA_NEO4J_PASSWORD").ok())
    else {
        eprintln!("SKIP: NEO4J_PASSWORD/DEKA_NEO4J_PASSWORD is not set");
        return;
    };

    let run_id = unique_id("platform_api_seam");
    let shop_a = format!("shop_{run_id}_a");
    let shop_b = format!("shop_{run_id}_b");
    let sku_a = format!("sku-{run_id}-a");
    let sku_b = format!("sku-{run_id}-b");

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build reqwest client");

    cleanup_fixture_rows(
        &client,
        &neo4j_password,
        &[&shop_a, &shop_b],
        &[&sku_a, &sku_b],
    )
    .await;
    seed_fixture_rows(&client, &neo4j_password, &shop_a, &shop_b, &sku_a, &sku_b).await;

    let port = free_port();
    let _platform = spawn_platform(port, &neo4j_password);
    wait_for_platform(&client, port).await;

    let products_a = get_json(&client, port, &shop_a, "/api/products", Some(&shop_b)).await;
    assert_eq!(products_a.status, 200, "body: {}", products_a.body);
    let rows = products_a
        .body
        .as_array()
        .expect("GET /api/products should return an array");
    assert!(
        rows.iter()
            .any(|row| row.get("sku").and_then(Value::as_str) == Some(sku_a.as_str())),
        "shop A product missing from scoped response: {}",
        products_a.body
    );
    assert!(
        rows.iter()
            .all(|row| row.get("sku").and_then(Value::as_str) != Some(sku_b.as_str())),
        "shop A request leaked shop B product despite conflicting bearer token: {}",
        products_a.body
    );

    let cross_sku = get_json(
        &client,
        port,
        &shop_a,
        &format!("/api/products/{sku_b}"),
        Some(&shop_b),
    )
    .await;
    assert_eq!(cross_sku.status, 404, "body: {}", cross_sku.body);
    assert_eq!(cross_sku.body["error"], "product not found");

    let own_sku = get_json(
        &client,
        port,
        &shop_b,
        &format!("/api/products/{sku_b}"),
        Some(&shop_b),
    )
    .await;
    assert_eq!(own_sku.status, 200, "body: {}", own_sku.body);
    assert_eq!(own_sku.body["sku"].as_str(), Some(sku_b.as_str()));

    cleanup_fixture_rows(
        &client,
        &neo4j_password,
        &[&shop_a, &shop_b],
        &[&sku_a, &sku_b],
    )
    .await;
}

struct JsonResponse {
    status: u16,
    body: Value,
}

async fn get_json(
    client: &Client,
    port: u16,
    host_shop: &str,
    path: &str,
    bearer_shop: Option<&str>,
) -> JsonResponse {
    let url = format!("http://127.0.0.1:{port}{path}");
    let mut request = client
        .get(url)
        .header("host", format!("{host_shop}.tana.gg"));
    if let Some(shop) = bearer_shop {
        request = request.header("authorization", format!("Bearer tenant:{shop}"));
    }
    let response = request.send().await.expect("send platform API request");
    let status = response.status().as_u16();
    let text = response.text().await.expect("read platform API body");
    let body = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("expected JSON response, got {err}: {text}"));
    JsonResponse { status, body }
}

fn spawn_platform(port: u16, neo4j_password: &str) -> PlatformProcess {
    let root = tempfile::tempdir().expect("create platform root");
    fs::create_dir_all(root.path().join("default")).expect("create default dir");
    fs::write(
        root.path().join("default/main.phpx"),
        r#"
struct StorefrontRequest {
    $url: string;
    $path: string;
    $pathname: string;
    $method: string;
    $body: Option<string>;
}

struct StorefrontResponse {
    $status: int;
    $body: string;
}

export function fetch($req: StorefrontRequest): StorefrontResponse {
    return StorefrontResponse { $status: 200, $body: "ok" };
}
"#,
    )
    .expect("write default handler");

    let child = Command::new(cli_bin())
        .arg("platform")
        .arg(root.path())
        .arg("--port")
        .arg(port.to_string())
        .env("DEKA_PLATFORM_API", "1")
        .env("DEKA_DEV_MODE", "1")
        .env("DEKA_NEO4J_URI", "bolt://127.0.0.1:7687")
        .env("DEKA_NEO4J_USER", "neo4j")
        .env("DEKA_NEO4J_PASSWORD", neo4j_password)
        .env("DEKA_NEO4J_DB", "neo4j")
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cli platform");

    PlatformProcess { child, _root: root }
}

async fn wait_for_platform(client: &Client, port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        let response = client
            .get(format!("http://127.0.0.1:{port}/api/shop"))
            .header("host", "missing-shop.tana.gg")
            .send()
            .await;
        if let Ok(response) = response {
            if response.status().as_u16() == 404 {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("platform did not become ready on port {port}");
}

async fn seed_fixture_rows(
    client: &Client,
    password: &str,
    shop_a: &str,
    shop_b: &str,
    sku_a: &str,
    sku_b: &str,
) {
    run_cypher(
        client,
        password,
        r#"
        CREATE (a:Shop {
          id: $shopA,
          name: 'Seam Tenant A',
          tagline: 'A',
          logoUrl: '',
          primaryColor: '#111111',
          mode: 'store'
        })-[:HAS]->(:Product {
          sku: $skuA,
          name: 'Only Tenant A Can See This',
          price: 101,
          description: 'tenant-a',
          category: 'seam',
          imageUrl: ''
        })
        CREATE (b:Shop {
          id: $shopB,
          name: 'Seam Tenant B',
          tagline: 'B',
          logoUrl: '',
          primaryColor: '#222222',
          mode: 'store'
        })-[:HAS]->(:Product {
          sku: $skuB,
          name: 'Only Tenant B Can See This',
          price: 202,
          description: 'tenant-b',
          category: 'seam',
          imageUrl: ''
        })
        "#,
        json!({
            "shopA": shop_a,
            "shopB": shop_b,
            "skuA": sku_a,
            "skuB": sku_b
        }),
    )
    .await;
}

async fn cleanup_fixture_rows(client: &Client, password: &str, shop_ids: &[&str], skus: &[&str]) {
    run_cypher(
        client,
        password,
        r#"
        MATCH (p:Product)
        WHERE p.sku IN $skus
        DETACH DELETE p
        "#,
        json!({ "skus": skus }),
    )
    .await;
    run_cypher(
        client,
        password,
        r#"
        MATCH (s:Shop)
        WHERE s.id IN $shopIds
        DETACH DELETE s
        "#,
        json!({ "shopIds": shop_ids }),
    )
    .await;
}

async fn run_cypher(client: &Client, password: &str, statement: &str, parameters: Value) {
    let response = client
        .post("http://127.0.0.1:7474/db/neo4j/tx/commit")
        .basic_auth("neo4j", Some(password))
        .json(&json!({
            "statements": [{
                "statement": statement,
                "parameters": parameters
            }]
        }))
        .send()
        .await
        .expect("send Neo4j query");
    let status = response.status();
    let body: Value = response.json().await.expect("parse Neo4j response");
    assert!(status.is_success(), "Neo4j HTTP status {status}: {}", body);
    assert_eq!(
        body.get("errors")
            .and_then(Value::as_array)
            .map(Vec::is_empty),
        Some(true),
        "Neo4j query errors: {}",
        body
    );
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read local addr")
        .port()
}

fn unique_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    format!("{prefix}_{}_{}", std::process::id(), nanos)
}
