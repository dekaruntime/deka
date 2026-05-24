use serde::{Deserialize, Serialize};

#[allow(clippy::too_many_arguments)]
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
