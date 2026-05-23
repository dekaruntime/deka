use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
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
use sqlx::FromRow;

fn unauthorized_response(
    message: &'static str,
) -> (
    StatusCode,
    [(header::HeaderName, &'static str); 1],
    &'static str,
) {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"linkhash\"")],
        message,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub user_id: i64,
    pub username: String,
    pub token_id: i64,
}

#[derive(Debug, FromRow)]
struct TokenRow {
    user_id: i64,
    username: String,
    token_id: i64,
}

#[derive(Debug, Serialize, FromRow)]
pub struct UserProfile {
    pub id: i64,
    pub username: String,
    pub email: Option<String>,
    pub status: String,
    pub email_verified_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
pub struct SignupResult {
    pub user: UserProfile,
    pub verification_required: bool,
    pub dev_bypass: bool,
}

#[derive(Debug, Serialize)]
pub struct LoginResult {
    pub token: String,
    pub user: UserProfile,
}

#[derive(Debug, Serialize, FromRow)]
pub struct SshKeyRecord {
    pub key_name: String,
    pub algorithm: String,
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug)]
struct ParsedCredentials {
    username_hint: Option<String>,
    token: String,
}

pub async fn require_auth(mut req: Request, next: Next) -> Result<Response, impl IntoResponse> {
    let parsed = match extract_credentials(&req) {
        Some(v) => v,
        None => {
            tracing::warn!("Missing credentials");
            return Err(unauthorized_response("Missing Authorization header"));
        }
    };

    let token_hash = sha256_hex(&parsed.token);
    let row = match lookup_token(&token_hash).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!("Unknown token");
            return Err(unauthorized_response("Invalid token"));
        }
        Err(e) => {
            tracing::error!("Auth lookup failed: {}", e);
            return Err(unauthorized_response("Authentication backend error"));
        }
    };

    if let Some(username_hint) = parsed.username_hint {
        if username_hint != row.username {
            tracing::warn!(
                "Basic username mismatch: supplied={}, token_owner={}",
                username_hint,
                row.username
            );
            return Err(unauthorized_response("Invalid username/token pair"));
        }
    }

    if let Err(e) = touch_token(row.token_id).await {
        tracing::warn!("Failed to update token last_used_at: {}", e);
    }

    req.extensions_mut().insert(AuthUser {
        user_id: row.user_id,
        username: row.username,
        token_id: row.token_id,
    });

    Ok(next.run(req).await)
}

pub fn get_auth_user(req: &Request) -> Option<&AuthUser> {
    req.extensions().get::<AuthUser>()
}

pub async fn auth_me(user_id: i64) -> Result<Option<UserProfile>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, UserProfile>(
        r#"
        SELECT id, username, email, status, email_verified_at
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn signup(
    username_raw: &str,
    email_raw: &str,
    password: &str,
    auto_verify_signups: bool,
) -> Result<SignupResult, anyhow::Error> {
    let username = normalize_username(username_raw)?;
    let email = normalize_email(email_raw)?;
    if password.trim().len() < 8 {
        anyhow::bail!("password must be at least 8 characters");
    }

    let hash = hash_password(password)?;
    let status = if auto_verify_signups {
        "active"
    } else {
        "pending"
    };

    let pool = crate::db::pool();
    let user = sqlx::query_as::<_, UserProfile>(
        r#"
        INSERT INTO users (username, email, password_hash, status, email_verified_at)
        VALUES ($1, $2, $3, $4, CASE WHEN $5 THEN NOW() ELSE NULL END)
        RETURNING id, username, email, status, email_verified_at
        "#,
    )
    .bind(&username)
    .bind(&email)
    .bind(&hash)
    .bind(status)
    .bind(auto_verify_signups)
    .fetch_one(pool)
    .await?;

    Ok(SignupResult {
        user,
        verification_required: !auto_verify_signups,
        dev_bypass: auto_verify_signups,
    })
}

pub async fn login(username_or_email: &str, password: &str) -> Result<LoginResult, anyhow::Error> {
    let identity = username_or_email.trim();
    if identity.is_empty() {
        anyhow::bail!("missing username or email");
    }
    let username_candidate = if identity.contains('@') {
        identity.to_string()
    } else {
        format!("@{}", identity)
    };

    let pool = crate::db::pool();
    let row = sqlx::query_as::<
        _,
        (
            i64,
            String,
            Option<String>,
            Option<String>,
            String,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        r#"
        SELECT id, username, email, password_hash, status, email_verified_at
        FROM users
        WHERE lower(username) = lower($1)
           OR lower(username) = lower($2)
           OR lower(email) = lower($1)
        LIMIT 1
        "#,
    )
    .bind(identity)
    .bind(&username_candidate)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("invalid credentials"))?;

    let (user_id, username, email, password_hash, status, email_verified_at) = row;
    if status == "suspended" {
        anyhow::bail!("account suspended");
    }
    let Some(hash) = password_hash else {
        anyhow::bail!("password login not enabled for this account");
    };
    if !verify_password(&hash, password)? {
        anyhow::bail!("invalid credentials");
    }

    let raw_token = generate_login_token();
    let token_hash = sha256_hex(&raw_token);
    let token_name = format!("login-{}", chrono::Utc::now().timestamp());
    sqlx::query(
        r#"
        INSERT INTO user_tokens (user_id, token_name, token_hash)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(user_id)
    .bind(token_name)
    .bind(token_hash)
    .execute(pool)
    .await?;

    let user = UserProfile {
        id: user_id,
        username,
        email,
        status,
        email_verified_at,
    };
    Ok(LoginResult {
        token: raw_token,
        user,
    })
}

async fn lookup_token(token_hash: &str) -> Result<Option<TokenRow>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, TokenRow>(
        r#"
        SELECT u.id AS user_id, u.username, t.id AS token_id
        FROM user_tokens t
        JOIN users u ON u.id = t.user_id
        WHERE t.token_hash = $1
          AND t.revoked_at IS NULL
          AND (t.expires_at IS NULL OR t.expires_at > NOW())
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

async fn touch_token(token_id: i64) -> Result<(), sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query("UPDATE user_tokens SET last_used_at = NOW() WHERE id = $1")
        .bind(token_id)
        .execute(pool)
        .await?;
    Ok(())
}

fn extract_credentials(req: &Request) -> Option<ParsedCredentials> {
    let auth_header = req.headers().get("authorization")?;
    let auth_str = auth_header.to_str().ok()?;

    if let Some(token) = auth_str.strip_prefix("Bearer ") {
        return Some(ParsedCredentials {
            username_hint: None,
            token: token.trim().to_string(),
        });
    }

    if let Some(b64) = auth_str.strip_prefix("Basic ") {
        let decoded = BASE64.decode(b64).ok()?;
        let credentials = String::from_utf8(decoded).ok()?;
        let parts: Vec<&str> = credentials.splitn(2, ':').collect();
        if parts.len() != 2 {
            return None;
        }

        return Some(ParsedCredentials {
            username_hint: Some(parts[0].to_string()),
            token: parts[1].to_string(),
        });
    }

    None
}

pub fn parse_ssh_public_key(raw_key: &str) -> Result<(String, String), &'static str> {
    let parts: Vec<&str> = raw_key.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("Invalid SSH key format");
    }

    let algorithm = parts[0].trim();
    if algorithm != "ssh-ed25519" {
        return Err("Only ssh-ed25519 keys are supported in MVP");
    }

    let key_blob = BASE64
        .decode(parts[1])
        .map_err(|_| "Invalid base64 in SSH key")?;

    let digest = Sha256::digest(&key_blob);
    let fingerprint = format!("SHA256:{}", BASE64.encode(digest).trim_end_matches('='));
    Ok((algorithm.to_string(), fingerprint))
}

pub async fn list_ssh_keys(user_id: i64) -> Result<Vec<SshKeyRecord>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, SshKeyRecord>(
        r#"
        SELECT key_name, algorithm, public_key, fingerprint, created_at
        FROM user_ssh_keys
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn add_ssh_key(
    user_id: i64,
    key_name: &str,
    raw_public_key: &str,
) -> Result<SshKeyRecord, anyhow::Error> {
    let (algorithm, fingerprint) =
        parse_ssh_public_key(raw_public_key).map_err(|e| anyhow::anyhow!(e))?;

    let pool = crate::db::pool();
    let inserted = sqlx::query_as::<_, SshKeyRecord>(
        r#"
        INSERT INTO user_ssh_keys (user_id, key_name, algorithm, public_key, fingerprint)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING key_name, algorithm, public_key, fingerprint, created_at
        "#,
    )
    .bind(user_id)
    .bind(key_name)
    .bind(&algorithm)
    .bind(raw_public_key)
    .bind(&fingerprint)
    .fetch_one(pool)
    .await?;

    Ok(inserted)
}

pub async fn delete_ssh_key(user_id: i64, fingerprint: &str) -> Result<bool, sqlx::Error> {
    let pool = crate::db::pool();
    let result = sqlx::query("DELETE FROM user_ssh_keys WHERE user_id = $1 AND fingerprint = $2")
        .bind(user_id)
        .bind(fingerprint)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

fn hash_password(password: &str) -> Result<String, anyhow::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hash failed: {}", e))?
        .to_string();
    Ok(hash)
}

fn verify_password(hash: &str, password: &str) -> Result<bool, anyhow::Error> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| anyhow::anyhow!("stored password hash invalid: {}", e))?;
    let argon2 = Argon2::default();
    Ok(argon2.verify_password(password.as_bytes(), &parsed).is_ok())
}

fn generate_login_token() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();
    format!("lh_pat_{}", suffix)
}

fn normalize_username(raw: &str) -> Result<String, anyhow::Error> {
    let trimmed = raw.trim();
    let candidate = if trimmed.starts_with('@') {
        trimmed.to_string()
    } else {
        format!("@{}", trimmed)
    };
    if candidate.len() < 2 {
        anyhow::bail!("username is required");
    }
    if !candidate[1..]
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("username contains invalid characters");
    }
    Ok(candidate)
}

fn normalize_email(raw: &str) -> Result<String, anyhow::Error> {
    let value = raw.trim().to_lowercase();
    if value.is_empty() || !value.contains('@') {
        anyhow::bail!("valid email is required");
    }
    Ok(value)
}
