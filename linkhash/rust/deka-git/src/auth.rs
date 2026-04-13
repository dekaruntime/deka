use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub token_id: i64,
    pub key_type: String,
    pub owner: String,
    pub scopes: Vec<String>,
    pub repos: Vec<String>,
}

impl AuthUser {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "*")
    }

    pub fn can_access_repo(&self, repo: &str) -> bool {
        self.repos.iter().any(|r| r == "*" || r == repo)
    }
}

fn unauthorized_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"tana-git\"")],
        message.to_string(),
    )
        .into_response()
}

pub async fn require_auth(mut req: Request, next: Next) -> Result<Response, Response> {
    let token = match extract_token(&req) {
        Some(t) => t,
        None => {
            return Err(unauthorized_response("Missing Authorization header"));
        }
    };

    let token_hash = sha256_hex(&token);
    let pool = crate::db::pool();

    let row = sqlx::query_as::<_, TokenRow>(
        r#"
        SELECT id, key_type, owner, scopes, repos, expires_at, revoked
        FROM access_tokens
        WHERE key_hash = ?
        "#,
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("Auth lookup failed: {}", e);
        unauthorized_response("Authentication backend error")
    })?;

    let row = match row {
        Some(r) => r,
        None => {
            return Err(unauthorized_response("Invalid token"));
        }
    };

    if row.revoked != 0 {
        return Err(unauthorized_response("Token revoked"));
    }

    if let Some(ref expires) = row.expires_at {
        if let Ok(exp) = chrono::NaiveDateTime::parse_from_str(expires, "%Y-%m-%d %H:%M:%S") {
            if exp < chrono::Utc::now().naive_utc() {
                return Err(unauthorized_response("Token expired"));
            }
        }
    }

    // Update last_used_at
    let _ = sqlx::query("UPDATE access_tokens SET last_used_at = datetime('now') WHERE id = ?")
        .bind(row.id)
        .execute(pool)
        .await;

    let scopes: Vec<String> =
        serde_json::from_str(&row.scopes).unwrap_or_else(|_| vec!["repo:read".to_string()]);
    let repos: Vec<String> =
        serde_json::from_str(&row.repos).unwrap_or_else(|_| vec!["*".to_string()]);

    req.extensions_mut().insert(AuthUser {
        token_id: row.id,
        key_type: row.key_type,
        owner: row.owner,
        scopes,
        repos,
    });

    Ok(next.run(req).await)
}

pub fn get_auth_user(req: &Request) -> Option<&AuthUser> {
    req.extensions().get::<AuthUser>()
}

// --- Token management ---

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

pub async fn create_token(req: CreateTokenRequest) -> anyhow::Result<CreateTokenResponse> {
    let raw_token = generate_token(&req.key_type);
    let token_hash = sha256_hex(&raw_token);
    let scopes = req
        .scopes
        .unwrap_or_else(|| vec!["repo:read".to_string()]);
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

// --- Audit logging ---

pub async fn log_audit(
    token_id: Option<i64>,
    key_type: &str,
    owner: &str,
    action: &str,
    repo: Option<&str>,
    ref_name: Option<&str>,
    detail: Option<&str>,
    ip: Option<&str>,
) {
    let pool = crate::db::pool();
    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO audit_log (token_id, key_type, owner, action, repo, ref_name, detail, ip)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(token_id)
    .bind(key_type)
    .bind(owner)
    .bind(action)
    .bind(repo)
    .bind(ref_name)
    .bind(detail)
    .bind(ip)
    .execute(pool)
    .await
    {
        tracing::error!("Failed to write audit log: {}", e);
    }
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub action: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: i64,
    pub token_id: Option<i64>,
    pub key_type: String,
    pub owner: String,
    pub action: String,
    pub repo: Option<String>,
    pub ref_name: Option<String>,
    pub detail: Option<String>,
    pub ip: Option<String>,
    pub timestamp: Option<String>,
}

pub async fn query_audit_log(query: &AuditQuery) -> anyhow::Result<Vec<AuditEntry>> {
    let pool = crate::db::pool();
    let limit = query.limit.unwrap_or(50).min(500);
    let offset = query.offset.unwrap_or(0);

    // Build dynamic query
    let mut sql = String::from("SELECT * FROM audit_log WHERE 1=1");
    let mut bind_values: Vec<String> = Vec::new();

    if let Some(ref action) = query.action {
        sql.push_str(&format!(" AND action = ?{}", bind_values.len() + 1));
        bind_values.push(action.clone());
    }
    if let Some(ref owner) = query.owner {
        sql.push_str(&format!(" AND owner = ?{}", bind_values.len() + 1));
        bind_values.push(owner.clone());
    }
    if let Some(ref repo) = query.repo {
        sql.push_str(&format!(" AND repo = ?{}", bind_values.len() + 1));
        bind_values.push(repo.clone());
    }

    // Use a simpler approach with manual binding
    let entries = match (&query.action, &query.owner, &query.repo) {
        (Some(action), Some(owner), Some(repo)) => {
            sqlx::query_as::<_, AuditEntry>(
                "SELECT * FROM audit_log WHERE action = ? AND owner = ? AND repo = ? ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(action)
            .bind(owner)
            .bind(repo)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (Some(action), Some(owner), None) => {
            sqlx::query_as::<_, AuditEntry>(
                "SELECT * FROM audit_log WHERE action = ? AND owner = ? ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(action)
            .bind(owner)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (Some(action), None, Some(repo)) => {
            sqlx::query_as::<_, AuditEntry>(
                "SELECT * FROM audit_log WHERE action = ? AND repo = ? ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(action)
            .bind(repo)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, Some(owner), Some(repo)) => {
            sqlx::query_as::<_, AuditEntry>(
                "SELECT * FROM audit_log WHERE owner = ? AND repo = ? ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(owner)
            .bind(repo)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (Some(action), None, None) => {
            sqlx::query_as::<_, AuditEntry>(
                "SELECT * FROM audit_log WHERE action = ? ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(action)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, Some(owner), None) => {
            sqlx::query_as::<_, AuditEntry>(
                "SELECT * FROM audit_log WHERE owner = ? ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(owner)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, None, Some(repo)) => {
            sqlx::query_as::<_, AuditEntry>(
                "SELECT * FROM audit_log WHERE repo = ? ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(repo)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, None, None) => {
            sqlx::query_as::<_, AuditEntry>(
                "SELECT * FROM audit_log ORDER BY id DESC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(entries)
}

// --- Helpers ---

#[derive(Debug, sqlx::FromRow)]
struct TokenRow {
    id: i64,
    key_type: String,
    owner: String,
    scopes: String,
    repos: String,
    expires_at: Option<String>,
    revoked: i32,
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

fn extract_token(req: &Request) -> Option<String> {
    let auth_header = req.headers().get("authorization")?;
    let auth_str = auth_header.to_str().ok()?;

    if let Some(token) = auth_str.strip_prefix("Bearer ") {
        return Some(token.trim().to_string());
    }

    // Support Basic auth for git clients (username is ignored, password is the token)
    if let Some(b64) = auth_str.strip_prefix("Basic ") {
        let decoded = BASE64.decode(b64).ok()?;
        let credentials = String::from_utf8(decoded).ok()?;
        let parts: Vec<&str> = credentials.splitn(2, ':').collect();
        if parts.len() == 2 {
            return Some(parts[1].to_string());
        }
    }

    None
}

pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
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

// --- Repo visibility ---

/// Get visibility for a repo. Defaults to "private" if not set.
pub async fn get_repo_visibility(owner: &str, name: &str) -> String {
    let pool = crate::db::pool();
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT visibility FROM repo_visibility WHERE repo_owner = ? AND repo_name = ?",
    )
    .bind(owner)
    .bind(name)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);

    row.map(|r| r.0).unwrap_or_else(|| "private".to_string())
}

/// Set visibility for a repo (upsert).
pub async fn set_repo_visibility(owner: &str, name: &str, visibility: &str) -> anyhow::Result<()> {
    let pool = crate::db::pool();
    sqlx::query(
        r#"
        INSERT INTO repo_visibility (repo_owner, repo_name, visibility)
        VALUES (?, ?, ?)
        ON CONFLICT(repo_owner, repo_name) DO UPDATE SET visibility = excluded.visibility
        "#,
    )
    .bind(owner)
    .bind(name)
    .bind(visibility)
    .execute(pool)
    .await?;
    Ok(())
}

/// Check if a repo is public.
pub async fn is_repo_public(owner: &str, name: &str) -> bool {
    get_repo_visibility(owner, name).await == "public"
}

/// Auth middleware that allows unauthenticated requests through (for public repo reads).
/// Attaches AuthUser to the request if a valid token is present, but does not reject
/// requests without a token.
pub async fn optional_auth(mut req: Request, next: Next) -> Response {
    if let Some(token) = extract_token(&req) {
        let token_hash = sha256_hex(&token);
        let pool = crate::db::pool();

        if let Ok(Some(row)) = sqlx::query_as::<_, TokenRow>(
            "SELECT id, key_type, owner, scopes, repos, expires_at, revoked FROM access_tokens WHERE key_hash = ?",
        )
        .bind(&token_hash)
        .fetch_optional(pool)
        .await
        {
            if row.revoked == 0 {
                let expired = if let Some(ref expires) = row.expires_at {
                    chrono::NaiveDateTime::parse_from_str(expires, "%Y-%m-%d %H:%M:%S")
                        .map(|exp| exp < chrono::Utc::now().naive_utc())
                        .unwrap_or(false)
                } else {
                    false
                };

                if !expired {
                    let _ = sqlx::query("UPDATE access_tokens SET last_used_at = datetime('now') WHERE id = ?")
                        .bind(row.id)
                        .execute(pool)
                        .await;

                    let scopes: Vec<String> =
                        serde_json::from_str(&row.scopes).unwrap_or_else(|_| vec!["repo:read".to_string()]);
                    let repos: Vec<String> =
                        serde_json::from_str(&row.repos).unwrap_or_else(|_| vec!["*".to_string()]);

                    req.extensions_mut().insert(AuthUser {
                        token_id: row.id,
                        key_type: row.key_type,
                        owner: row.owner,
                        scopes,
                        repos,
                    });
                }
            }
        }
    }

    next.run(req).await
}

/// Seed default labels for a repo (idempotent)
pub async fn seed_labels(repo_owner: &str, repo_name: &str) {
    let labels = [
        ("bug", "e11d48"),
        ("feature", "3b82f6"),
        ("security", "f97316"),
        ("transpiler", "a855f7"),
        ("storefront", "22c55e"),
        ("infra", "9ca3af"),
        ("next-apps", "06b6d4"),
        ("tests", "eab308"),
        ("blocked", "e11d48"),
        ("in-progress", "3b82f6"),
        ("needs-review", "eab308"),
    ];

    let pool = crate::db::pool();
    for (name, color) in labels {
        let _ = sqlx::query(
            "INSERT OR IGNORE INTO labels (repo_owner, repo_name, name, color) VALUES (?, ?, ?, ?)",
        )
        .bind(repo_owner)
        .bind(repo_name)
        .bind(name)
        .bind(color)
        .execute(pool)
        .await;
    }
}
