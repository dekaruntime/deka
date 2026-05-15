use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod audit;
mod labels;
mod tokens;
mod visibility;

#[allow(unused_imports)]
pub use audit::AuditEntry;
pub use audit::{log_audit, query_audit_log, AuditQuery};
pub use labels::seed_labels;
pub use tokens::{create_token, list_tokens, revoke_token, CreateTokenRequest};
#[allow(unused_imports)]
pub use tokens::{CreateTokenResponse, TokenInfo};
pub use visibility::{get_repo_visibility, is_repo_public, set_repo_visibility};

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

// --- Helpers ---

#[derive(Debug, sqlx::FromRow)]
pub(super) struct TokenRow {
    id: i64,
    key_type: String,
    owner: String,
    scopes: String,
    repos: String,
    expires_at: Option<String>,
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
