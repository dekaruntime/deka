use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};

use super::{sha256_hex, AuthUser};
use crate::authz::{RepoAccess, RepoGrant};
use harar_client::{Secrets, SecretsError, VaultClient};

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

#[derive(Debug, sqlx::FromRow)]
struct TokenMigrationRow {
    id: i64,
    key_hash: String,
    key_type: String,
    owner: String,
    scopes: String,
    repos: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenMigrationSummary {
    pub migrated: usize,
    pub skipped: usize,
    pub failed: usize,
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

pub async fn migrate_tokens_to_vault(
    socket_path: impl AsRef<std::path::Path>,
) -> anyhow::Result<TokenMigrationSummary> {
    let secrets = Secrets::from_socket_path(socket_path.as_ref());
    let vault = VaultClient::from_socket_path(socket_path);
    migrate_tokens_to_vault_with_clients(crate::db::pool(), &secrets, &vault).await
}

pub async fn migrate_tokens_to_vault_with_clients(
    pool: &sqlx::SqlitePool,
    secrets: &Secrets,
    vault: &VaultClient,
) -> anyhow::Result<TokenMigrationSummary> {
    let rows = sqlx::query_as::<_, TokenMigrationRow>(
        r#"
        SELECT id, key_hash, key_type, owner, scopes, repos
        FROM access_tokens
        WHERE revoked = 0
          AND (expires_at IS NULL OR expires_at > datetime('now'))
        ORDER BY id
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut summary = TokenMigrationSummary::default();
    for row in &rows {
        let key = match row.vault_key() {
            Ok(key) => key,
            Err(err) => {
                summary.failed += 1;
                tracing::error!(owner = %row.owner, error = %err, "failed to derive token vault key");
                continue;
            }
        };

        match secrets.get(&key).await {
            Ok(_) => {
                summary.skipped += 1;
                tracing::info!(key = %key, username = %row.owner, "skipped existing token vault key");
                continue;
            }
            Err(SecretsError::AgentStatus { status: 404, .. }) => {}
            Err(err) => {
                summary.failed += 1;
                tracing::error!(key = %key, username = %row.owner, error = %err, "failed to check token vault key");
                continue;
            }
        }

        let value = match serde_json::to_string(&row.to_auth_user()) {
            Ok(value) => value,
            Err(err) => {
                summary.failed += 1;
                tracing::error!(key = %key, username = %row.owner, error = %err, "failed to serialize token vault payload");
                continue;
            }
        };

        match vault.put(&key, &value).await {
            Ok(()) => {
                summary.migrated += 1;
                tracing::info!(key = %key, username = %row.owner, "migrated token vault key");
            }
            Err(err) => {
                summary.failed += 1;
                tracing::error!(key = %key, username = %row.owner, error = %err, "failed to migrate token vault key");
            }
        }
    }

    Ok(summary)
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

impl TokenMigrationRow {
    fn vault_key(&self) -> anyhow::Result<String> {
        let Some(hash_prefix) = self.key_hash.get(..16) else {
            anyhow::bail!("token hash for owner {} is too short", self.owner);
        };
        Ok(format!("GIT_TOKEN_{hash_prefix}"))
    }

    fn to_auth_user(&self) -> AuthUser {
        let scopes: Vec<String> = serde_json::from_str(&self.scopes).unwrap_or_default();
        let repos: Vec<String> = serde_json::from_str(&self.repos).unwrap_or_default();
        let access = if scopes
            .iter()
            .any(|scope| scope == "*" || scope == "repo:write")
        {
            RepoAccess::Write
        } else {
            RepoAccess::Read
        };

        AuthUser {
            token_id: self.id,
            key_type: self.key_type.clone(),
            owner: self.owner.clone(),
            scopes,
            repos: repos.clone(),
            repo_grants: repos
                .into_iter()
                .map(|repo| RepoGrant {
                    repo,
                    access: access.clone(),
                })
                .collect(),
            secret_grants: Vec::new(),
        }
    }
}
