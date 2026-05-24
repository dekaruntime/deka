use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    sync::atomic::{AtomicBool, Ordering},
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;

use crate::authz::{self, RepoAccess, RepoGrant, SecretGrant};

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

type HmacSha256 = Hmac<Sha256>;

const DEFAULT_AUTH_CACHE_TTL_SECONDS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub token_id: i64,
    pub key_type: String,
    pub owner: String,
    pub scopes: Vec<String>,
    pub repos: Vec<String>,
    pub repo_grants: Vec<RepoGrant>,
    pub secret_grants: Vec<SecretGrant>,
}

impl AuthUser {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "*")
    }

    pub fn can_access_repo(&self, repo: &str) -> bool {
        authz::can_read_repo(&self.repos, &self.repo_grants, repo)
    }

    pub fn can_write_repo(&self, repo: &str) -> bool {
        authz::can_write_repo(&self.repos, &self.repo_grants, repo)
    }

    pub fn can_read_secret(&self, repo: &str, secret_name: &str) -> bool {
        authz::can_read_secret(&self.secret_grants, repo, secret_name)
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
    let auth_user = resolve_auth_user(&token, &token_hash)
        .await
        .map_err(|e| {
            tracing::error!("Auth lookup failed: {}", e);
            unauthorized_response("Authentication backend error")
        })?
        .ok_or_else(|| unauthorized_response("Invalid token"))?;

    req.extensions_mut().insert(auth_user);

    Ok(next.run(req).await)
}

pub fn get_auth_user(req: &Request) -> Option<&AuthUser> {
    req.extensions().get::<AuthUser>()
}

// --- Helpers ---

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

#[derive(Clone)]
struct CachedAuthUser {
    user: AuthUser,
    expires_at: Instant,
}

#[derive(Debug, Serialize)]
struct AdminAuthRequest<'a> {
    token: &'a str,
}

#[derive(Debug, Deserialize)]
struct AdminAuthResponse {
    username: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    repo_grants: Vec<AdminRepoGrant>,
    #[serde(default)]
    secret_grants: Vec<AdminSecretGrant>,
}

#[derive(Debug, Deserialize)]
struct AdminRepoGrant {
    repo: String,
    #[serde(default = "default_repo_access")]
    access: String,
}

#[derive(Debug, Deserialize)]
struct AdminSecretGrant {
    repo: String,
    #[serde(alias = "secret_pattern")]
    pattern: String,
}

static AUTH_CACHE: OnceLock<RwLock<HashMap<String, CachedAuthUser>>> = OnceLock::new();
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static ADMIN_AUTH_UNCONFIGURED_LOGGED: AtomicBool = AtomicBool::new(false);

fn auth_cache() -> &'static RwLock<HashMap<String, CachedAuthUser>> {
    AUTH_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(reqwest::Client::new)
}

async fn resolve_auth_user(
    token: &str,
    token_hash: &str,
) -> Result<Option<AuthUser>, anyhow::Error> {
    if let Some(user) = cached_auth_user(token_hash).await {
        return Ok(Some(user));
    }

    let Some(user) = fetch_auth_user_from_admin(token).await? else {
        return Ok(None);
    };

    let ttl = auth_cache_ttl();
    if !ttl.is_zero() {
        auth_cache().write().await.insert(
            token_hash.to_string(),
            CachedAuthUser {
                user: user.clone(),
                expires_at: Instant::now() + ttl,
            },
        );
    }

    Ok(Some(user))
}

async fn cached_auth_user(token_hash: &str) -> Option<AuthUser> {
    let now = Instant::now();
    {
        let cache = auth_cache().read().await;
        if let Some(cached) = cache.get(token_hash) {
            if cached.expires_at > now {
                return Some(cached.user.clone());
            }
        }
    }

    auth_cache().write().await.remove(token_hash);
    None
}

async fn fetch_auth_user_from_admin(token: &str) -> Result<Option<AuthUser>, anyhow::Error> {
    let hmac_key = match std::env::var("TANA_INTERNAL_HMAC_KEY") {
        Ok(key) => key,
        Err(_) => {
            if !ADMIN_AUTH_UNCONFIGURED_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::info!(
                    "tana-admin auth integration not configured (TANA_INTERNAL_HMAC_KEY unset)"
                );
            }
            return Ok(None);
        }
    };
    let admin_url = std::env::var("TANA_ADMIN_URL")
        .unwrap_or_else(|_| "http://localhost:3000".to_string())
        .trim_end_matches('/')
        .to_string();
    let body = serde_json::to_vec(&AdminAuthRequest { token })?;
    let timestamp = unix_timestamp_seconds();
    let signature = sign_internal_request(&hmac_key, timestamp, &body)?;
    let endpoint = format!("{}/api/internal/auth/by-token", admin_url);

    let response = http_client()
        .post(endpoint)
        .header("content-type", "application/json")
        .header("x-tana-timestamp", timestamp.to_string())
        .header("x-tana-signature", signature)
        .body(body)
        .send()
        .await?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
        || response.status() == reqwest::StatusCode::NOT_FOUND
    {
        return Ok(None);
    }

    if !response.status().is_success() {
        anyhow::bail!("tana-admin auth returned {}", response.status());
    }

    let admin_user = response.json::<AdminAuthResponse>().await?;
    Ok(Some(admin_user.into_auth_user()))
}

impl AdminAuthResponse {
    fn into_auth_user(self) -> AuthUser {
        let repo_grants: Vec<RepoGrant> = self
            .repo_grants
            .into_iter()
            .map(|grant| RepoGrant {
                repo: grant.repo,
                access: parse_repo_access(&grant.access),
            })
            .collect();
        let secret_grants: Vec<SecretGrant> = self
            .secret_grants
            .into_iter()
            .map(|grant| SecretGrant {
                repo: grant.repo,
                pattern: grant.pattern,
            })
            .collect();
        let scopes = derive_scopes(&self.labels, &repo_grants, &secret_grants);
        let repos = repo_grants
            .iter()
            .map(|grant| grant.repo.clone())
            .collect::<Vec<_>>();
        let key_type = derive_key_type(&self.labels);

        AuthUser {
            token_id: 0,
            key_type,
            owner: self.username,
            scopes,
            repos,
            repo_grants,
            secret_grants,
        }
    }
}

fn auth_cache_ttl() -> Duration {
    let seconds = std::env::var("TANA_GIT_AUTH_CACHE_TTL_SECONDS")
        .or_else(|_| std::env::var("TANA_AUTH_CACHE_TTL_SECONDS"))
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_AUTH_CACHE_TTL_SECONDS);
    Duration::from_secs(seconds)
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs()
}

fn sign_internal_request(
    hmac_key: &str,
    timestamp: u64,
    body: &[u8],
) -> Result<String, hmac::digest::InvalidLength> {
    let mut mac = HmacSha256::new_from_slice(hmac_key.as_bytes())?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn derive_key_type(labels: &[String]) -> String {
    if labels.iter().any(|label| label.eq_ignore_ascii_case("agent")) {
        "agent".to_string()
    } else {
        "user".to_string()
    }
}

fn parse_repo_access(access: &str) -> RepoAccess {
    if access.eq_ignore_ascii_case("write") {
        RepoAccess::Write
    } else {
        RepoAccess::Read
    }
}

fn default_repo_access() -> String {
    "read".to_string()
}

fn derive_scopes(
    labels: &[String],
    repo_grants: &[RepoGrant],
    secret_grants: &[SecretGrant],
) -> Vec<String> {
    if labels.iter().any(|label| {
        label.eq_ignore_ascii_case("admin")
            || label.eq_ignore_ascii_case("system")
            || label.eq_ignore_ascii_case("root")
    }) {
        return vec!["*".to_string()];
    }

    let mut scopes = BTreeSet::new();
    if !repo_grants.is_empty() {
        scopes.insert("repo:read".to_string());
        scopes.insert("packages:read".to_string());
    }
    if repo_grants
        .iter()
        .any(|grant| grant.access == RepoAccess::Write)
    {
        scopes.insert("repo:write".to_string());
        scopes.insert("issues:write".to_string());
        scopes.insert("packages:write".to_string());
    }
    if !secret_grants.is_empty() {
        scopes.insert("secrets:read".to_string());
    }

    scopes.into_iter().collect()
}

/// Auth middleware that allows unauthenticated requests through (for public repo reads).
/// Attaches AuthUser to the request if a valid token is present, but does not reject
/// requests without a token.
pub async fn optional_auth(mut req: Request, next: Next) -> Response {
    if let Some(token) = extract_token(&req) {
        let token_hash = sha256_hex(&token);
        match resolve_auth_user(&token, &token_hash).await {
            Ok(Some(auth_user)) => {
                req.extensions_mut().insert(auth_user);
            }
            Ok(None) => {}
            Err(e) => tracing::error!("Optional auth lookup failed: {}", e),
        }
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_auth_user_from_admin_noops_without_hmac_key() {
        let previous_hmac_key = std::env::var_os("TANA_INTERNAL_HMAC_KEY");
        std::env::remove_var("TANA_INTERNAL_HMAC_KEY");

        let user = fetch_auth_user_from_admin("test-token").await;

        assert!(user.expect("missing hmac key should not error").is_none());

        if let Some(previous_hmac_key) = previous_hmac_key {
            std::env::set_var("TANA_INTERNAL_HMAC_KEY", previous_hmac_key);
        }
    }
}
