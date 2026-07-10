use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use gild_vault_client::{Secrets, SecretsError};
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
pub use tokens::{
    create_token, list_tokens, migrate_tokens_to_vault, revoke_token, CreateTokenRequest,
};
#[allow(unused_imports)]
pub use tokens::{migrate_tokens_to_vault_with_clients, CreateTokenResponse, TokenInfo};
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
static ADMIN_AUTH_KEY_SOURCE_LOGGED: AtomicBool = AtomicBool::new(false);

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
    let vault = Secrets::from_socket()?;
    resolve_auth_user_with_vault(token, token_hash, &vault).await
}

async fn resolve_auth_user_with_vault(
    token: &str,
    token_hash: &str,
    vault: &Secrets,
) -> Result<Option<AuthUser>, anyhow::Error> {
    if let Some(user) = cached_auth_user(token_hash).await {
        tracing::debug!(source = "cache", "auth user resolved");
        return Ok(Some(user));
    }

    let user = if let Some(user) = fetch_auth_user_from_vault_with_vault(token_hash, vault).await? {
        tracing::debug!(source = "vault", "auth user resolved");
        user
    } else if let Some(user) = fetch_auth_user_from_admin_with_vault(token, vault).await? {
        tracing::debug!(source = "admin", "auth user resolved");
        user
    } else {
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

async fn fetch_auth_user_from_vault_with_vault(
    token_hash: &str,
    vault: &Secrets,
) -> Result<Option<AuthUser>, anyhow::Error> {
    let Some(hash_prefix) = token_hash.get(..16) else {
        anyhow::bail!("token hash is too short for vault auth lookup");
    };
    let key = format!("GIT_TOKEN_{hash_prefix}");

    let value = match vault.get(&key).await {
        Ok(value) => value,
        Err(SecretsError::AgentStatus { status: 404, .. }) => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let user = match serde_json::from_str::<AuthUser>(&value) {
        Ok(user) => user,
        Err(_) => serde_json::from_str::<AdminAuthResponse>(&value)?.into_auth_user(),
    };
    Ok(Some(user))
}

async fn fetch_auth_user_from_admin_with_vault(
    token: &str,
    vault: &Secrets,
) -> Result<Option<AuthUser>, anyhow::Error> {
    let Some(hmac_key) = resolve_internal_hmac_key_with_vault(vault).await else {
        return Ok(None);
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

#[derive(Debug, Clone, Copy)]
enum HmacKeySource {
    Vault,
    Env,
    None,
}

async fn resolve_internal_hmac_key_with_vault(vault: &Secrets) -> Option<String> {
    match vault.get("TANA_INTERNAL_HMAC_KEY").await {
        Ok(key) => {
            log_admin_auth_key_source(HmacKeySource::Vault);
            return Some(key);
        }
        Err(SecretsError::AgentStatus { status: 404, .. }) => {}
        Err(err) => {
            tracing::debug!("vault lookup for TANA_INTERNAL_HMAC_KEY failed: {}", err);
        }
    }

    match std::env::var("TANA_INTERNAL_HMAC_KEY") {
        Ok(key) => {
            log_admin_auth_key_source(HmacKeySource::Env);
            Some(key)
        }
        Err(_) => {
            log_admin_auth_key_source(HmacKeySource::None);
            None
        }
    }
}

fn log_admin_auth_key_source(source: HmacKeySource) {
    if ADMIN_AUTH_KEY_SOURCE_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }

    match source {
        HmacKeySource::Vault => {
            tracing::info!(source = "vault", "tana-admin auth integration configured")
        }
        HmacKeySource::Env => tracing::info!(
            source = "env",
            "tana-admin auth integration configured from env fallback"
        ),
        HmacKeySource::None => tracing::info!(
            source = "none",
            "tana-admin auth integration not configured (TANA_INTERNAL_HMAC_KEY missing)"
        ),
    }
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
    if labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case("agent"))
    {
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
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
            Arc,
        },
    };
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, UnixListener},
        sync::Mutex,
    };

    #[tokio::test]
    async fn vault_hit_returns_user_from_vault() {
        clear_auth_cache().await;
        let token_hash = sha256_hex("vault-token");
        let vault_key = format!("GIT_TOKEN_{}", &token_hash[..16]);
        let vault_value = serde_json::json!({
            "username": "vault-user",
            "labels": ["agent"],
            "repo_grants": [{ "repo": "tana/deka", "access": "write" }],
            "secret_grants": []
        })
        .to_string();
        let vault = MockVault::start(&[(&vault_key, &vault_value)]).await;

        let user = fetch_auth_user_from_vault_with_vault(
            &token_hash,
            &Secrets::from_socket_path(&vault.socket_path),
        )
        .await
        .expect("vault lookup should not error")
        .expect("vault should return auth user");

        assert_eq!(user.owner, "vault-user");
        assert!(user.can_write_repo("tana/deka"));
        assert_eq!(vault.requests.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn vault_miss_admin_hit_returns_user_from_admin() {
        let _guard = env_lock().lock().await;
        let _env = EnvGuard::capture(&[
            "TANA_INTERNAL_HMAC_KEY",
            "TANA_ADMIN_URL",
            "TANA_GIT_AUTH_CACHE_TTL_SECONDS",
        ]);
        clear_auth_cache().await;
        std::env::remove_var("TANA_INTERNAL_HMAC_KEY");
        std::env::set_var("TANA_GIT_AUTH_CACHE_TTL_SECONDS", "0");

        let vault = MockVault::start(&[("TANA_INTERNAL_HMAC_KEY", "vault-secret")]).await;
        let admin = MockAdmin::start("vault-secret").await;
        std::env::set_var("TANA_ADMIN_URL", &admin.url);

        let user = resolve_auth_user_with_vault(
            "admin-token",
            &sha256_hex("admin-token"),
            &Secrets::from_socket_path(&vault.socket_path),
        )
        .await
        .expect("auth lookup should not error")
        .expect("admin fallback should return auth user");

        assert_eq!(user.owner, "vault-user");
        assert_eq!(admin.requests.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn missing_git_token_in_vault_falls_through_to_admin() {
        let _guard = env_lock().lock().await;
        let _env = EnvGuard::capture(&[
            "TANA_INTERNAL_HMAC_KEY",
            "TANA_ADMIN_URL",
            "TANA_GIT_AUTH_CACHE_TTL_SECONDS",
        ]);
        clear_auth_cache().await;
        std::env::remove_var("TANA_INTERNAL_HMAC_KEY");
        std::env::set_var("TANA_GIT_AUTH_CACHE_TTL_SECONDS", "0");

        let token_hash = sha256_hex("admin-token");
        let vault = MockVault::start(&[("TANA_INTERNAL_HMAC_KEY", "vault-secret")]).await;
        let admin = MockAdmin::start("vault-secret").await;
        std::env::set_var("TANA_ADMIN_URL", &admin.url);

        let user = resolve_auth_user_with_vault(
            "admin-token",
            &token_hash,
            &Secrets::from_socket_path(&vault.socket_path),
        )
        .await
        .expect("vault 404 should be treated as auth miss")
        .expect("admin fallback should return auth user");

        assert_eq!(user.owner, "vault-user");
        assert_eq!(admin.requests.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(vault.requests.load(AtomicOrdering::SeqCst), 2);
    }

    #[tokio::test]
    async fn vault_and_admin_miss_returns_none() {
        let _guard = env_lock().lock().await;
        let _env = EnvGuard::capture(&[
            "TANA_INTERNAL_HMAC_KEY",
            "TANA_ADMIN_URL",
            "TANA_GIT_AUTH_CACHE_TTL_SECONDS",
        ]);
        clear_auth_cache().await;
        std::env::remove_var("TANA_INTERNAL_HMAC_KEY");
        std::env::remove_var("TANA_ADMIN_URL");
        std::env::set_var("TANA_GIT_AUTH_CACHE_TTL_SECONDS", "0");
        let vault = MockVault::start(&[]).await;

        let user = resolve_auth_user_with_vault(
            "missing-token",
            &sha256_hex("missing-token"),
            &Secrets::from_socket_path(&vault.socket_path),
        )
        .await
        .expect("auth miss should not error");

        assert!(user.is_none());
    }

    #[tokio::test]
    async fn cache_hit_returns_immediately() {
        clear_auth_cache().await;
        let token_hash = sha256_hex("cached-token");
        auth_cache().write().await.insert(
            token_hash.clone(),
            CachedAuthUser {
                user: AuthUser {
                    token_id: 7,
                    key_type: "agent".to_string(),
                    owner: "cached-user".to_string(),
                    scopes: vec!["repo:read".to_string()],
                    repos: vec!["tana/deka".to_string()],
                    repo_grants: Vec::new(),
                    secret_grants: Vec::new(),
                },
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        let temp = TempDir::new().unwrap();
        let missing_vault = Secrets::from_socket_path(temp.path().join("missing.sock"));

        let user = resolve_auth_user_with_vault("cached-token", &token_hash, &missing_vault)
            .await
            .expect("cache hit should not touch vault")
            .expect("cache should return auth user");

        assert_eq!(user.owner, "cached-user");
    }

    #[tokio::test]
    async fn fetch_auth_user_from_admin_uses_vault_hmac_key() {
        let _guard = env_lock().lock().await;
        let _env = EnvGuard::capture(&["TANA_INTERNAL_HMAC_KEY", "TANA_ADMIN_URL"]);
        std::env::remove_var("TANA_INTERNAL_HMAC_KEY");
        let vault = MockVault::start(&[("TANA_INTERNAL_HMAC_KEY", "vault-secret")]).await;
        let admin = MockAdmin::start("vault-secret").await;
        std::env::set_var("TANA_ADMIN_URL", &admin.url);

        let user = fetch_auth_user_from_admin_with_vault(
            "test-token",
            &Secrets::from_socket_path(&vault.socket_path),
        )
        .await
        .expect("admin fetch should not error")
        .expect("vault hmac key should authenticate admin request");

        assert_eq!(user.owner, "vault-user");
        assert_eq!(admin.requests.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fetch_auth_user_from_admin_falls_back_to_env_hmac_key() {
        let _guard = env_lock().lock().await;
        let _env = EnvGuard::capture(&["TANA_INTERNAL_HMAC_KEY", "TANA_ADMIN_URL"]);
        std::env::set_var("TANA_INTERNAL_HMAC_KEY", "env-secret");
        let vault = MockVault::start(&[]).await;
        let admin = MockAdmin::start("env-secret").await;
        std::env::set_var("TANA_ADMIN_URL", &admin.url);

        let user = fetch_auth_user_from_admin_with_vault(
            "test-token",
            &Secrets::from_socket_path(&vault.socket_path),
        )
        .await
        .expect("admin fetch should not error")
        .expect("env hmac key should authenticate admin request");

        assert_eq!(user.owner, "vault-user");
        assert_eq!(admin.requests.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fetch_auth_user_from_admin_noops_without_hmac_key() {
        let _guard = env_lock().lock().await;
        let _env = EnvGuard::capture(&["TANA_INTERNAL_HMAC_KEY", "TANA_ADMIN_URL"]);
        std::env::remove_var("TANA_INTERNAL_HMAC_KEY");
        std::env::remove_var("TANA_ADMIN_URL");
        let vault = MockVault::start(&[]).await;

        let user = fetch_auth_user_from_admin_with_vault(
            "test-token",
            &Secrets::from_socket_path(&vault.socket_path),
        )
        .await;

        assert!(user.expect("missing hmac key should not error").is_none());
    }

    fn env_lock() -> &'static Mutex<()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    async fn clear_auth_cache() {
        auth_cache().write().await.clear();
    }

    struct EnvGuard {
        values: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn capture(keys: &[&'static str]) -> Self {
            Self {
                values: keys
                    .iter()
                    .map(|key| (*key, std::env::var_os(key)))
                    .collect(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.values {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    struct MockVault {
        socket_path: PathBuf,
        _temp: TempDir,
        requests: Arc<AtomicUsize>,
    }

    impl MockVault {
        async fn start(secrets: &[(&str, &str)]) -> Self {
            let temp = TempDir::new().unwrap();
            let socket_path = temp.path().join("vault.sock");
            let listener = UnixListener::bind(&socket_path).unwrap();
            let secrets = Arc::new(
                secrets
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                    .collect::<HashMap<_, _>>(),
            );
            let requests = Arc::new(AtomicUsize::new(0));

            tokio::spawn({
                let secrets = Arc::clone(&secrets);
                let requests = Arc::clone(&requests);
                async move {
                    loop {
                        let Ok((mut stream, _)) = listener.accept().await else {
                            return;
                        };
                        let secrets = Arc::clone(&secrets);
                        let requests = Arc::clone(&requests);
                        tokio::spawn(async move {
                            requests.fetch_add(1, AtomicOrdering::SeqCst);
                            let mut request = Vec::new();
                            let _ = stream.read_to_end(&mut request).await;
                            let key = vault_request_key(&request).unwrap_or_default();
                            let (status, body) = match secrets.get(&key) {
                                Some(value) => ("200 OK", serde_json::json!({ "value": value })),
                                None => {
                                    ("404 Not Found", serde_json::json!({ "error": "missing" }))
                                }
                            };
                            let body = body.to_string();
                            let response = format!(
                                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.shutdown().await;
                        });
                    }
                }
            });

            Self {
                socket_path,
                _temp: temp,
                requests,
            }
        }
    }

    fn vault_request_key(request: &[u8]) -> Option<String> {
        let request = std::str::from_utf8(request).ok()?;
        let path = request.split_whitespace().nth(1)?;
        path.strip_prefix("/v1/secret/").map(ToOwned::to_owned)
    }

    struct MockAdmin {
        url: String,
        requests: Arc<AtomicUsize>,
    }

    impl MockAdmin {
        async fn start(expected_hmac_key: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(AtomicUsize::new(0));

            tokio::spawn({
                let requests = Arc::clone(&requests);
                async move {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    requests.fetch_add(1, AtomicOrdering::SeqCst);
                    let request = read_http_request(&mut stream).await;
                    let status = if admin_request_signature_is_valid(&request, expected_hmac_key) {
                        "200 OK"
                    } else {
                        "401 Unauthorized"
                    };
                    let body = serde_json::json!({
                        "username": "vault-user",
                        "labels": ["agent"],
                        "repo_grants": [{ "repo": "tana/deka", "access": "write" }],
                        "secret_grants": []
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            });

            Self {
                url: format!("http://{addr}"),
                requests,
            }
        }
    }

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        let mut content_length = None;
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if content_length.is_none() {
                if let Some(header_end) = find_header_end(&request) {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                }
            }
            if let (Some(header_end), Some(content_length)) =
                (find_header_end(&request), content_length)
            {
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        request
    }

    fn admin_request_signature_is_valid(request: &[u8], expected_hmac_key: &str) -> bool {
        let Some(header_end) = find_header_end(request) else {
            return false;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let body = &request[header_end + 4..];
        let mut timestamp = None;
        let mut signature = None;
        for line in headers.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("x-tana-timestamp") {
                timestamp = value.trim().parse::<u64>().ok();
            }
            if name.eq_ignore_ascii_case("x-tana-signature") {
                signature = Some(value.trim().to_string());
            }
        }

        let Some(timestamp) = timestamp else {
            return false;
        };
        let expected = sign_internal_request(expected_hmac_key, timestamp, body).unwrap();
        signature.as_deref() == Some(expected.as_str())
    }

    fn find_header_end(request: &[u8]) -> Option<usize> {
        request.windows(4).position(|window| window == b"\r\n\r\n")
    }
}
