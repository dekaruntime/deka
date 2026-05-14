use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::redaction;

#[derive(Debug, Serialize, FromRow)]
pub struct PullRequest {
    pub id: i64,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub author: String,
    pub source_ref: String,
    pub target_ref: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub closed_at: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct PullComment {
    pub id: i64,
    pub pull_id: i64,
    pub body: String,
    pub author: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePullRequest {
    pub title: String,
    pub body: Option<String>,
    pub source_ref: String,
    pub target_ref: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePullRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListPullsQuery {
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePullComment {
    pub body: String,
}

async fn get_next_pull_number(repo_owner: &str, repo_name: &str) -> Result<i64, sqlx::Error> {
    let pool = crate::db::pool();

    sqlx::query(
        r#"
        INSERT INTO pull_sequences (repo_owner, repo_name, next_number)
        VALUES (?, ?, 2)
        ON CONFLICT (repo_owner, repo_name)
        DO UPDATE SET next_number = pull_sequences.next_number + 1
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .execute(pool)
    .await?;

    let result: (i64,) = sqlx::query_as(
        "SELECT next_number - 1 FROM pull_sequences WHERE repo_owner = ? AND repo_name = ?",
    )
    .bind(repo_owner)
    .bind(repo_name)
    .fetch_one(pool)
    .await?;

    Ok(result.0)
}

pub async fn create_pull(
    repo_owner: &str,
    repo_name: &str,
    author: &str,
    req: CreatePullRequest,
) -> Result<PullRequest, sqlx::Error> {
    let number = get_next_pull_number(repo_owner, repo_name).await?;
    let pool = crate::db::pool();
    let redacted_title = redaction::redact_secret_strings(req.title.trim());
    redaction::log_redactions("pull.title.create", &redacted_title.redactions);
    let (redacted_body, body_redactions) = redaction::redact_optional_secret_strings(
        req.body.map(|value| value.trim().to_string()),
    );
    redaction::log_redactions("pull.body.create", &body_redactions);
    sqlx::query_as::<_, PullRequest>(
        r#"
        INSERT INTO pull_requests (repo_owner, repo_name, number, title, body, state, author, source_ref, target_ref)
        VALUES (?, ?, ?, ?, ?, 'open', ?, ?, ?)
        RETURNING *
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .bind(redacted_title.text)
    .bind(redacted_body)
    .bind(author)
    .bind(req.source_ref.trim())
    .bind(req.target_ref.trim())
    .fetch_one(pool)
    .await
}

pub async fn list_pulls(
    repo_owner: &str,
    repo_name: &str,
    query: &ListPullsQuery,
) -> Result<Vec<PullRequest>, sqlx::Error> {
    let state = query.state.as_deref().unwrap_or("open");
    let pool = crate::db::pool();
    sqlx::query_as::<_, PullRequest>(
        r#"
        SELECT * FROM pull_requests
        WHERE repo_owner = ? AND repo_name = ? AND (? = 'all' OR state = ?)
        ORDER BY number DESC
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(state)
    .bind(state)
    .fetch_all(pool)
    .await
}

pub async fn get_pull(
    repo_owner: &str,
    repo_name: &str,
    number: i64,
) -> Result<Option<PullRequest>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, PullRequest>(
        "SELECT * FROM pull_requests WHERE repo_owner = ? AND repo_name = ? AND number = ?",
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_optional(pool)
    .await
}

pub async fn update_pull(
    repo_owner: &str,
    repo_name: &str,
    number: i64,
    req: UpdatePullRequest,
) -> Result<Option<PullRequest>, sqlx::Error> {
    let Some(existing) = get_pull(repo_owner, repo_name, number).await? else {
        return Ok(None);
    };
    let title_raw = req.title.unwrap_or(existing.title);
    let body_raw = req.body.or(existing.body);
    let title = redaction::redact_secret_strings(&title_raw);
    redaction::log_redactions("pull.title.update", &title.redactions);
    let (body, body_redactions) = redaction::redact_optional_secret_strings(body_raw);
    redaction::log_redactions("pull.body.update", &body_redactions);
    let state = req.state.unwrap_or(existing.state);
    let closed_at = if state == "closed" || state == "merged" {
        Some(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string())
    } else {
        None
    };
    let pool = crate::db::pool();
    sqlx::query_as::<_, PullRequest>(
        r#"
        UPDATE pull_requests
        SET title = ?, body = ?, state = ?, updated_at = datetime('now'), closed_at = ?
        WHERE repo_owner = ? AND repo_name = ? AND number = ?
        RETURNING *
        "#,
    )
    .bind(title.text)
    .bind(body)
    .bind(state)
    .bind(closed_at)
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_optional(pool)
    .await
}

pub async fn add_pull_comment(
    repo_owner: &str,
    repo_name: &str,
    number: i64,
    author: &str,
    req: CreatePullComment,
) -> Result<Option<PullComment>, sqlx::Error> {
    let Some(pr) = get_pull(repo_owner, repo_name, number).await? else {
        return Ok(None);
    };
    let pool = crate::db::pool();
    let redacted_body = redaction::redact_secret_strings(req.body.trim());
    redaction::log_redactions("pull.comment.create", &redacted_body.redactions);
    let comment = sqlx::query_as::<_, PullComment>(
        "INSERT INTO pull_comments (pull_id, body, author) VALUES (?, ?, ?) RETURNING *",
    )
    .bind(pr.id)
    .bind(redacted_body.text)
    .bind(author)
    .fetch_one(pool)
    .await?;
    sqlx::query("UPDATE pull_requests SET updated_at = datetime('now') WHERE id = ?")
        .bind(pr.id)
        .execute(pool)
        .await?;
    Ok(Some(comment))
}

pub async fn list_pull_comments(
    repo_owner: &str,
    repo_name: &str,
    number: i64,
) -> Result<Vec<PullComment>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, PullComment>(
        r#"
        SELECT c.* FROM pull_comments c
        JOIN pull_requests p ON c.pull_id = p.id
        WHERE p.repo_owner = ? AND p.repo_name = ? AND p.number = ?
        ORDER BY c.created_at ASC
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_all(pool)
    .await
}
