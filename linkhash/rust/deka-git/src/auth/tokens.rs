use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};

use super::sha256_hex;

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    pub key_type: String,
    pub owner: String,
    pub scopes: Option<Vec<String>>,
    pub repos: Option<Vec<String>>,
    pub expires_in_days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateTokenResponse {
    pub id: i64,
    pub token: String,
    pub key_type: String,
    pub owner: String,
    pub scopes: Vec<String>,
    pub repos: Vec<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenInfo {
    pub id: i64,
    pub key_type: String,
    pub owner: String,
    pub scopes: Vec<String>,
    pub repos: Vec<String>,
    pub created_at: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct TokenListRow {
    id: i64,
    key_type: String,
    owner: String,
    scopes: String,
    repos: String,
    created_at: Option<String>,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    revoked: i32,
}

pub async fn create_token(req: CreateTokenRequest) -> anyhow::Result<CreateTokenResponse> {
    let raw_token = generate_token(&req.key_type);
    let token_hash = sha256_hex(&raw_token);
    let scopes = req.scopes.unwrap_or_else(|| vec!["repo:read".to_string()]);
    let repos = req.repos.unwrap_or_else(|| vec!["*".to_string()]);
    let scopes_json = serde_json::to_string(&scopes)?;
    let repos_json = serde_json::to_string(&repos)?;

    let expires_at = req.expires_in_days.map(|days| {
        let exp = chrono::Utc::now() + chrono::Duration::days(days);
        exp.format("%Y-%m-%d %H:%M:%S").to_string()
    });

    let pool = crate::db::pool();
    let id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO access_tokens (key_hash, key_type, owner, scopes, repos, expires_at)
        VALUES (?, ?, ?, ?, ?, ?)
        RETURNING id
        "#,
    )
    .bind(&token_hash)
    .bind(&req.key_type)
    .bind(&req.owner)
    .bind(&scopes_json)
    .bind(&repos_json)
    .bind(&expires_at)
    .fetch_one(pool)
    .await?;

    Ok(CreateTokenResponse {
        id,
        token: raw_token,
        key_type: req.key_type,
        owner: req.owner,
        scopes,
        repos,
        expires_at,
    })
}

pub async fn list_tokens() -> anyhow::Result<Vec<TokenInfo>> {
    let pool = crate::db::pool();
    let rows = sqlx::query_as::<_, TokenListRow>(
        "SELECT id, key_type, owner, scopes, repos, created_at, expires_at, last_used_at, revoked FROM access_tokens ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| TokenInfo {
            id: r.id,
            key_type: r.key_type,
            owner: r.owner,
            scopes: serde_json::from_str(&r.scopes).unwrap_or_default(),
            repos: serde_json::from_str(&r.repos).unwrap_or_default(),
            created_at: r.created_at,
            expires_at: r.expires_at,
            last_used_at: r.last_used_at,
            revoked: r.revoked != 0,
        })
        .collect())
}

pub async fn revoke_token(token_id: i64) -> anyhow::Result<bool> {
    let pool = crate::db::pool();
    let result = sqlx::query("UPDATE access_tokens SET revoked = 1 WHERE id = ?")
        .bind(token_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

fn generate_token(key_type: &str) -> String {
    let prefix = match key_type {
        "system" => "tg_sys",
        "agent" => "tg_agt",
        "user" => "tg_usr",
        _ => "tg_unk",
    };
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();
    format!("{}_{}", prefix, suffix)
}
