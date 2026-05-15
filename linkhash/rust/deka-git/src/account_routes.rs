use axum::{
    extract::{Extension, Path, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth;

const AGENT_ACCOUNTS: &[(&str, &str)] = &[
    ("idris", "idris@agents.tana.local"),
    ("layla", "layla@agents.tana.local"),
    ("khalid", "khalid@agents.tana.local"),
    ("yasmin", "yasmin@agents.tana.local"),
    ("mariam", "mariam@agents.tana.local"),
    ("tariq", "tariq@agents.tana.local"),
    ("noor", "noor@agents.tana.local"),
    ("hamza", "hamza@agents.tana.local"),
    ("amina", "amina@agents.tana.local"),
    ("samira", "samira@agents.tana.local"),
    ("huda", "huda@agents.tana.local"),
    ("sami", "sami@tana.local"),
];

#[derive(Debug, Serialize, sqlx::FromRow)]
pub(crate) struct Account {
    username: String,
    email: String,
    account_type: String,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SshKeyPayload {
    key_name: String,
    public_key: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RepoAclPayload {
    account: String,
    access: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SecretAclPayload {
    account: String,
    pattern: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WebhookPayload {
    url: String,
    events: Option<Vec<String>>,
}

pub(crate) async fn handle_auth_me(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "owner": auth_user.owner.clone(),
            "key_type": auth_user.key_type.clone(),
            "scopes": auth_user.scopes.clone(),
            "legacy_repos": auth_user.repos.clone(),
            "repo_acl": auth_user.repo_grants.clone(),
            "secret_acl": auth_user.secret_grants.clone(),
        })),
    )
}

pub(crate) async fn handle_bootstrap_agents(req: Request) -> impl IntoResponse {
    let auth_user = match require_admin(&req) {
        Ok(user) => user,
        Err(response) => return response,
    };

    let pool = crate::db::pool();
    for (username, email) in AGENT_ACCOUNTS {
        if let Err(e) = sqlx::query(
            r#"
            INSERT INTO accounts (username, email, account_type, display_name)
            VALUES (?, ?, 'agent', ?)
            ON CONFLICT(username) DO UPDATE SET
                email = excluded.email,
                account_type = excluded.account_type,
                display_name = excluded.display_name,
                updated_at = datetime('now')
            "#,
        )
        .bind(*username)
        .bind(*email)
        .bind(title_case(username))
        .execute(pool)
        .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            );
        }
    }

    auth::log_audit(
        Some(auth_user.token_id),
        &auth_user.key_type,
        &auth_user.owner,
        "accounts.bootstrap_agents",
        None,
        None,
        Some("seeded first-class agent accounts"),
        None,
    )
    .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({ "accounts_seeded": AGENT_ACCOUNTS.len() })),
    )
}

pub(crate) async fn handle_list_accounts(req: Request) -> impl IntoResponse {
    if let Err(response) = require_admin(&req) {
        return response;
    }

    match sqlx::query_as::<_, Account>(
        "SELECT username, email, account_type, display_name FROM accounts ORDER BY username",
    )
    .fetch_all(crate::db::pool())
    .await
    {
        Ok(accounts) => (StatusCode::OK, Json(serde_json::json!({ "accounts": accounts }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_register_ssh_key(
    Path(account): Path<String>,
    Extension(auth_user): Extension<auth::AuthUser>,
    Json(payload): Json<SshKeyPayload>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_user(&auth_user) {
        return response;
    }

    let fingerprint = ssh_fingerprint(&payload.public_key);
    let pool = crate::db::pool();

    let account_id = match sqlx::query_scalar::<_, i64>("SELECT id FROM accounts WHERE username = ?")
        .bind(&account)
        .fetch_optional(pool)
        .await
    {
        Ok(Some(id)) => id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Account not found" })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    let result = sqlx::query(
        r#"
        INSERT INTO account_ssh_keys (account_id, key_name, public_key, fingerprint)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(public_key) DO UPDATE SET
            key_name = excluded.key_name,
            revoked = 0
        "#,
    )
    .bind(account_id)
    .bind(&payload.key_name)
    .bind(&payload.public_key)
    .bind(&fingerprint)
    .execute(pool)
    .await;

    match result {
        Ok(_) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "accounts.ssh_key.register",
                None,
                None,
                Some(&format!("account={} fingerprint={}", account, fingerprint)),
                None,
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "account": account, "fingerprint": fingerprint })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_grant_repo_acl(
    Path((owner, repo)): Path<(String, String)>,
    Extension(auth_user): Extension<auth::AuthUser>,
    Json(payload): Json<RepoAclPayload>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_user(&auth_user) {
        return response;
    }

    if payload.access != "read" && payload.access != "write" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "access must be read or write" })),
        );
    }

    match sqlx::query(
        r#"
        INSERT INTO repo_acl (account, repo_owner, repo_name, access, granted_by)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(account, repo_owner, repo_name) DO UPDATE SET
            access = excluded.access,
            granted_by = excluded.granted_by
        "#,
    )
    .bind(&payload.account)
    .bind(&owner)
    .bind(&repo)
    .bind(&payload.access)
    .bind(&auth_user.owner)
    .execute(crate::db::pool())
    .await
    {
        Ok(_) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "repo_acl.grant",
                Some(&format!("{}/{}", owner, repo)),
                None,
                Some(&format!("account={} access={}", payload.account, payload.access)),
                None,
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "account": payload.account, "repo": format!("{}/{}", owner, repo), "access": payload.access })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_grant_secret_acl(
    Path((owner, repo)): Path<(String, String)>,
    Extension(auth_user): Extension<auth::AuthUser>,
    Json(payload): Json<SecretAclPayload>,
) -> impl IntoResponse {
    if let Err(response) = require_admin_user(&auth_user) {
        return response;
    }

    match sqlx::query(
        r#"
        INSERT INTO secret_acl (account, repo_owner, repo_name, secret_pattern, granted_by)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(account, repo_owner, repo_name, secret_pattern) DO UPDATE SET
            granted_by = excluded.granted_by
        "#,
    )
    .bind(&payload.account)
    .bind(&owner)
    .bind(&repo)
    .bind(&payload.pattern)
    .bind(&auth_user.owner)
    .execute(crate::db::pool())
    .await
    {
        Ok(_) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "secret_acl.grant",
                Some(&format!("{}/{}", owner, repo)),
                None,
                Some(&format!("account={} pattern={}", payload.account, payload.pattern)),
                None,
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "account": payload.account, "repo": format!("{}/{}", owner, repo), "pattern": payload.pattern })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_env_acl(
    Path((owner, repo)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };
    let full_repo = format!("{}/{}", owner, repo);
    if !auth_user.has_scope("secrets:read")
        || (!auth_user.can_access_repo(&full_repo) && !auth_user.can_access_repo(&repo))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "secrets:read and repo read access required" })),
        );
    }

    let patterns: Vec<String> = auth_user
        .secret_grants
        .iter()
        .filter(|grant| grant.repo == full_repo || grant.repo == "*")
        .map(|grant| grant.pattern.clone())
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "repo": full_repo,
            "account": auth_user.owner,
            "authorized_secret_patterns": patterns,
            "values": {},
            "storage": "vault",
        })),
    )
}

pub(crate) async fn handle_create_webhook(
    Path((owner, repo)): Path<(String, String)>,
    Extension(auth_user): Extension<auth::AuthUser>,
    Json(payload): Json<WebhookPayload>,
) -> impl IntoResponse {
    let full_repo = format!("{}/{}", owner, repo);
    if !auth_user.has_scope("repo:write")
        || (!auth_user.can_write_repo(&full_repo) && !auth_user.can_write_repo(&repo))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "repo:write access required" })),
        );
    }
    let events = payload.events.unwrap_or_else(|| vec!["push".to_string()]);
    let events_json = match serde_json::to_string(&events) {
        Ok(events) => events,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match sqlx::query(
        r#"
        INSERT INTO webhook_subscriptions (repo_owner, repo_name, url, events, created_by)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(repo_owner, repo_name, url) DO UPDATE SET
            events = excluded.events,
            active = 1
        "#,
    )
    .bind(&owner)
    .bind(&repo)
    .bind(&payload.url)
    .bind(&events_json)
    .bind(&auth_user.owner)
    .execute(crate::db::pool())
    .await
    {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "repo": full_repo, "url": payload.url, "events": events })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_list_webhooks(
    Path((owner, repo)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };
    let full_repo = format!("{}/{}", owner, repo);
    if !auth_user.has_scope("repo:read")
        || (!auth_user.can_access_repo(&full_repo) && !auth_user.can_access_repo(&repo))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "repo:read access required" })),
        );
    }

    match sqlx::query_as::<_, WebhookRow>(
        "SELECT id, url, events, active FROM webhook_subscriptions WHERE repo_owner = ? AND repo_name = ? ORDER BY id",
    )
    .bind(&owner)
    .bind(&repo)
    .fetch_all(crate::db::pool())
    .await
    {
        Ok(webhooks) => (
            StatusCode::OK,
            Json(serde_json::json!({ "repo": full_repo, "webhooks": webhooks })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct WebhookRow {
    id: i64,
    url: String,
    events: String,
    active: i64,
}

fn require_admin(req: &Request) -> Result<&auth::AuthUser, (StatusCode, Json<serde_json::Value>)> {
    let auth_user = auth::get_auth_user(req).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Authentication required" })),
        )
    })?;

    require_admin_user(auth_user)?;

    Ok(auth_user)
}

fn require_admin_user(
    auth_user: &auth::AuthUser,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if !auth_user.has_scope("admin:write") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "admin:write scope required" })),
        ));
    }

    Ok(())
}

fn ssh_fingerprint(public_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key.trim().as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
