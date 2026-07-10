use gild_vault_client::{Secrets, VaultClient};
use linkhash::auth::{migrate_tokens_to_vault_with_clients, sha256_hex, AuthUser};
use sqlx::sqlite::SqlitePoolOptions;
use std::{collections::HashMap, sync::Arc};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::Mutex,
};

#[tokio::test]
async fn migrates_sqlite_tokens_to_unix_socket_vault_idempotently() -> anyhow::Result<()> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    create_access_tokens_table(&pool).await?;

    let tokens = [
        (
            "tg_usr_alpha",
            "user",
            "alice",
            vec!["repo:read"],
            vec!["tana/deka"],
        ),
        (
            "tg_agt_bravo",
            "agent",
            "agent-khalid",
            vec!["repo:write"],
            vec!["tana/deka", "tana/tana"],
        ),
        ("tg_sys_charlie", "system", "system", vec!["*"], vec!["*"]),
    ];

    for (token, key_type, owner, scopes, repos) in &tokens {
        sqlx::query(
            r#"
            INSERT INTO access_tokens (key_hash, key_type, owner, scopes, repos, revoked)
            VALUES (?, ?, ?, ?, ?, 0)
            "#,
        )
        .bind(sha256_hex(token))
        .bind(*key_type)
        .bind(*owner)
        .bind(serde_json::to_string(&scopes)?)
        .bind(serde_json::to_string(&repos)?)
        .execute(&pool)
        .await?;
    }

    let vault = MockVault::start().await?;
    let secrets = Secrets::from_socket_path(&vault.socket_path);
    let client = VaultClient::from_socket_path(&vault.socket_path);

    let summary = migrate_tokens_to_vault_with_clients(&pool, &secrets, &client).await?;
    assert_eq!(summary.migrated, 3);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.failed, 0);

    let stored = vault.keys.lock().await.clone();
    assert_eq!(stored.len(), 3);
    for (token, _key_type, owner, _scopes, _repos) in tokens {
        let key = format!("GIT_TOKEN_{}", &sha256_hex(token)[..16]);
        let value = stored.get(&key).expect("token should be stored in vault");
        let user = serde_json::from_str::<AuthUser>(value)?;
        assert_eq!(user.owner, owner);
        assert!(user.token_id > 0);
    }

    let summary = migrate_tokens_to_vault_with_clients(&pool, &secrets, &client).await?;
    assert_eq!(summary.migrated, 0);
    assert_eq!(summary.skipped, 3);
    assert_eq!(summary.failed, 0);
    assert_eq!(vault.keys.lock().await.len(), 3);

    Ok(())
}

async fn create_access_tokens_table(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE access_tokens (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            key_hash TEXT NOT NULL UNIQUE,
            key_type TEXT NOT NULL,
            owner TEXT NOT NULL,
            scopes TEXT NOT NULL,
            repos TEXT NOT NULL,
            expires_at TEXT,
            revoked INTEGER DEFAULT 0
        )
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

struct MockVault {
    socket_path: std::path::PathBuf,
    keys: Arc<Mutex<HashMap<String, String>>>,
    _temp: TempDir,
}

impl MockVault {
    async fn start() -> anyhow::Result<Self> {
        let temp = TempDir::new()?;
        let socket_path = temp.path().join("vault.sock");
        let listener = UnixListener::bind(&socket_path)?;
        let keys = Arc::new(Mutex::new(HashMap::<String, String>::new()));

        tokio::spawn({
            let keys = Arc::clone(&keys);
            async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    let keys = Arc::clone(&keys);
                    tokio::spawn(async move {
                        let mut request = Vec::new();
                        if stream.read_to_end(&mut request).await.is_err() {
                            return;
                        }

                        let response = if request.starts_with(b"GET ") {
                            handle_http_get(&keys, &request).await
                        } else {
                            handle_json_request(&keys, &request).await
                        };

                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    });
                }
            }
        });

        Ok(Self {
            socket_path,
            keys,
            _temp: temp,
        })
    }
}

async fn handle_http_get(keys: &Mutex<HashMap<String, String>>, request: &[u8]) -> String {
    let key = std::str::from_utf8(request)
        .ok()
        .and_then(|request| request.split_whitespace().nth(1))
        .and_then(|path| path.strip_prefix("/v1/secret/"))
        .unwrap_or_default()
        .to_string();
    let keys = keys.lock().await;
    match keys.get(&key) {
        Some(value) => http_response("200 OK", serde_json::json!({ "value": value })),
        None => http_response("404 Not Found", serde_json::json!({ "error": "missing" })),
    }
}

async fn handle_json_request(keys: &Mutex<HashMap<String, String>>, request: &[u8]) -> String {
    let parsed = serde_json::from_slice::<serde_json::Value>(request);
    let Ok(parsed) = parsed else {
        return serde_json::json!({ "ok": false, "error": "bad_request" }).to_string();
    };
    if parsed.get("op").and_then(|op| op.as_str()) != Some("put") {
        return serde_json::json!({ "ok": false, "error": "unsupported" }).to_string();
    }
    let Some(key) = parsed.get("key").and_then(|key| key.as_str()) else {
        return serde_json::json!({ "ok": false, "error": "missing_key" }).to_string();
    };
    let Some(value) = parsed.get("value").and_then(|value| value.as_str()) else {
        return serde_json::json!({ "ok": false, "error": "missing_value" }).to_string();
    };

    keys.lock().await.insert(key.to_string(), value.to_string());
    serde_json::json!({ "ok": true }).to_string()
}

fn http_response(status: &str, body: serde_json::Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}
