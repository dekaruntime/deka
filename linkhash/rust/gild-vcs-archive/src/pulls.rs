use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, FromRow)]
pub struct PullRequest {
    pub id: i32,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: i32,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub author: String,
    pub source_ref: String,
    pub target_ref: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct PullComment {
    pub id: i32,
    pub pull_id: i32,
    pub body: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
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

async fn get_next_pull_number(repo_owner: &str, repo_name: &str) -> Result<i32, sqlx::Error> {
    let pool = crate::db::pool();
    let row: (i32,) = sqlx::query_as(
        r#"
        INSERT INTO pull_sequences (repo_owner, repo_name, next_number)
        VALUES ($1, $2, 2)
        ON CONFLICT (repo_owner, repo_name)
        DO UPDATE SET next_number = pull_sequences.next_number + 1
        RETURNING next_number - 1
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn create_pull(
    repo_owner: &str,
    repo_name: &str,
    author: &str,
    req: CreatePullRequest,
) -> Result<PullRequest, sqlx::Error> {
    let number = get_next_pull_number(repo_owner, repo_name).await?;
    let pool = crate::db::pool();
    sqlx::query_as::<_, PullRequest>(
        r#"
        INSERT INTO pull_requests (repo_owner, repo_name, number, title, body, state, author, source_ref, target_ref)
        VALUES ($1, $2, $3, $4, $5, 'open', $6, $7, $8)
        RETURNING *
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .bind(req.title.trim())
    .bind(req.body.map(|v| v.trim().to_string()))
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
        WHERE repo_owner = $1 AND repo_name = $2 AND ($3 = 'all' OR state = $3)
        ORDER BY number DESC
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(state)
    .fetch_all(pool)
    .await
}

pub async fn get_pull(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
) -> Result<Option<PullRequest>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, PullRequest>(
        r#"
        SELECT * FROM pull_requests
        WHERE repo_owner = $1 AND repo_name = $2 AND number = $3
        "#,
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
    number: i32,
    req: UpdatePullRequest,
) -> Result<Option<PullRequest>, sqlx::Error> {
    let Some(existing) = get_pull(repo_owner, repo_name, number).await? else {
        return Ok(None);
    };
    let title = req.title.unwrap_or(existing.title);
    let body = req.body.or(existing.body);
    let state = req.state.unwrap_or(existing.state);
    let closed_at = if state == "closed" {
        Some(Utc::now())
    } else {
        None
    };
    let pool = crate::db::pool();
    sqlx::query_as::<_, PullRequest>(
        r#"
        UPDATE pull_requests
        SET title = $4, body = $5, state = $6, updated_at = NOW(), closed_at = $7
        WHERE repo_owner = $1 AND repo_name = $2 AND number = $3
        RETURNING *
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .bind(title)
    .bind(body)
    .bind(state)
    .bind(closed_at)
    .fetch_optional(pool)
    .await
}

pub async fn add_pull_comment(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
    author: &str,
    req: CreatePullComment,
) -> Result<Option<PullComment>, sqlx::Error> {
    let Some(pr) = get_pull(repo_owner, repo_name, number).await? else {
        return Ok(None);
    };
    let pool = crate::db::pool();
    let comment = sqlx::query_as::<_, PullComment>(
        r#"
        INSERT INTO pull_comments (pull_id, body, author)
        VALUES ($1, $2, $3)
        RETURNING *
        "#,
    )
    .bind(pr.id)
    .bind(req.body.trim())
    .bind(author)
    .fetch_one(pool)
    .await?;
    sqlx::query("UPDATE pull_requests SET updated_at = NOW() WHERE id = $1")
        .bind(pr.id)
        .execute(pool)
        .await?;
    Ok(Some(comment))
}

pub async fn list_pull_comments(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
) -> Result<Vec<PullComment>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, PullComment>(
        r#"
        SELECT c.* FROM pull_comments c
        JOIN pull_requests p ON c.pull_id = p.id
        WHERE p.repo_owner = $1 AND p.repo_name = $2 AND p.number = $3
        ORDER BY c.created_at ASC
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_all(pool)
    .await
}
