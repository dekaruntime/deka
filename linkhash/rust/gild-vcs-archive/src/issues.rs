use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Issue {
    pub id: i32,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: i32,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct IssueComment {
    pub id: i32,
    pub issue_id: i32,
    pub body: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Label {
    pub id: i32,
    pub repo_owner: String,
    pub repo_name: String,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateIssueRequest {
    pub title: String,
    pub body: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateIssueRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommentRequest {
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct ListIssuesQuery {
    pub state: Option<String>, // open, closed, all
    #[allow(dead_code)]
    pub author: Option<String>,
    #[allow(dead_code)]
    pub label: Option<String>,
}

/// Get next issue number for a repo (auto-increment)
async fn get_next_issue_number(repo_owner: &str, repo_name: &str) -> Result<i32, sqlx::Error> {
    let pool = db::pool();

    // Insert or update sequence, returning the number
    let result: (i32,) = sqlx::query_as(
        r#"
        INSERT INTO issue_sequences (repo_owner, repo_name, next_number)
        VALUES ($1, $2, 2)
        ON CONFLICT (repo_owner, repo_name)
        DO UPDATE SET next_number = issue_sequences.next_number + 1
        RETURNING next_number - 1
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .fetch_one(pool)
    .await?;

    Ok(result.0)
}

/// Create a new issue
pub async fn create_issue(
    repo_owner: &str,
    repo_name: &str,
    author: &str,
    req: CreateIssueRequest,
) -> Result<Issue, sqlx::Error> {
    let pool = db::pool();
    let number = get_next_issue_number(repo_owner, repo_name).await?;

    let issue = sqlx::query_as::<_, Issue>(
        r#"
        INSERT INTO issues (repo_owner, repo_name, number, title, body, author)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING *
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .bind(&req.title)
    .bind(&req.body)
    .bind(author)
    .fetch_one(pool)
    .await?;

    Ok(issue)
}

/// List issues for a repository
pub async fn list_issues(
    repo_owner: &str,
    repo_name: &str,
    query: &ListIssuesQuery,
) -> Result<Vec<Issue>, sqlx::Error> {
    let pool = db::pool();

    let state_filter = match query.state.as_deref() {
        Some("closed") => "closed",
        Some("all") => "%",
        _ => "open",
    };

    let issues = sqlx::query_as::<_, Issue>(
        r#"
        SELECT * FROM issues
        WHERE repo_owner = $1 AND repo_name = $2
        AND state LIKE $3
        ORDER BY number DESC
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(state_filter)
    .fetch_all(pool)
    .await?;

    Ok(issues)
}

/// Get a single issue by number
pub async fn get_issue(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
) -> Result<Option<Issue>, sqlx::Error> {
    let pool = db::pool();

    let issue = sqlx::query_as::<_, Issue>(
        r#"
        SELECT * FROM issues
        WHERE repo_owner = $1 AND repo_name = $2 AND number = $3
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_optional(pool)
    .await?;

    Ok(issue)
}

/// Update an issue
pub async fn update_issue(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
    req: UpdateIssueRequest,
) -> Result<Option<Issue>, sqlx::Error> {
    let pool = db::pool();

    // Get existing issue
    let existing = get_issue(repo_owner, repo_name, number).await?;
    let existing = match existing {
        Some(i) => i,
        None => return Ok(None),
    };

    let existing_state = existing.state.clone();
    let new_title = req.title.unwrap_or(existing.title);
    let new_body = req.body.or(existing.body);
    let new_state = req.state.unwrap_or(existing_state.clone());

    let closed_at = if new_state == "closed" && existing_state != "closed" {
        Some(Utc::now())
    } else if new_state == "open" {
        None
    } else {
        existing.closed_at
    };

    let issue = sqlx::query_as::<_, Issue>(
        r#"
        UPDATE issues
        SET title = $4, body = $5, state = $6, closed_at = $7, updated_at = NOW()
        WHERE repo_owner = $1 AND repo_name = $2 AND number = $3
        RETURNING *
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .bind(&new_title)
    .bind(&new_body)
    .bind(&new_state)
    .bind(closed_at)
    .fetch_optional(pool)
    .await?;

    Ok(issue)
}

/// Close an issue
#[allow(dead_code)]
pub async fn close_issue(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
) -> Result<Option<Issue>, sqlx::Error> {
    update_issue(
        repo_owner,
        repo_name,
        number,
        UpdateIssueRequest {
            title: None,
            body: None,
            state: Some("closed".to_string()),
        },
    )
    .await
}

/// Reopen an issue
#[allow(dead_code)]
pub async fn reopen_issue(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
) -> Result<Option<Issue>, sqlx::Error> {
    update_issue(
        repo_owner,
        repo_name,
        number,
        UpdateIssueRequest {
            title: None,
            body: None,
            state: Some("open".to_string()),
        },
    )
    .await
}

/// Add a comment to an issue
pub async fn add_comment(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
    author: &str,
    req: CreateCommentRequest,
) -> Result<Option<IssueComment>, sqlx::Error> {
    let pool = db::pool();

    // Get issue ID
    let issue = get_issue(repo_owner, repo_name, number).await?;
    let issue = match issue {
        Some(i) => i,
        None => return Ok(None),
    };

    let comment = sqlx::query_as::<_, IssueComment>(
        r#"
        INSERT INTO issue_comments (issue_id, body, author)
        VALUES ($1, $2, $3)
        RETURNING *
    "#,
    )
    .bind(issue.id)
    .bind(&req.body)
    .bind(author)
    .fetch_one(pool)
    .await?;

    // Update issue timestamp
    sqlx::query("UPDATE issues SET updated_at = NOW() WHERE id = $1")
        .bind(issue.id)
        .execute(pool)
        .await?;

    Ok(Some(comment))
}

/// List comments on an issue
pub async fn list_comments(
    repo_owner: &str,
    repo_name: &str,
    number: i32,
) -> Result<Vec<IssueComment>, sqlx::Error> {
    let pool = db::pool();

    let comments = sqlx::query_as::<_, IssueComment>(
        r#"
        SELECT c.* FROM issue_comments c
        JOIN issues i ON c.issue_id = i.id
        WHERE i.repo_owner = $1 AND i.repo_name = $2 AND i.number = $3
        ORDER BY c.created_at ASC
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_all(pool)
    .await?;

    Ok(comments)
}

// Label operations

/// Create a label
pub async fn create_label(
    repo_owner: &str,
    repo_name: &str,
    name: &str,
    color: &str,
    description: Option<&str>,
) -> Result<Label, sqlx::Error> {
    let pool = db::pool();

    let label = sqlx::query_as::<_, Label>(
        r#"
        INSERT INTO labels (repo_owner, repo_name, name, color, description)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(name)
    .bind(color)
    .bind(description)
    .fetch_one(pool)
    .await?;

    Ok(label)
}

/// List labels for a repo
pub async fn list_labels(repo_owner: &str, repo_name: &str) -> Result<Vec<Label>, sqlx::Error> {
    let pool = db::pool();

    let labels = sqlx::query_as::<_, Label>(
        r#"
        SELECT * FROM labels
        WHERE repo_owner = $1 AND repo_name = $2
        ORDER BY name
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .fetch_all(pool)
    .await?;

    Ok(labels)
}

/// Add label to issue
pub async fn add_label_to_issue(
    repo_owner: &str,
    repo_name: &str,
    issue_number: i32,
    label_name: &str,
) -> Result<bool, sqlx::Error> {
    let pool = db::pool();

    // Get issue and label IDs
    let issue = get_issue(repo_owner, repo_name, issue_number).await?;
    let issue = match issue {
        Some(i) => i,
        None => return Ok(false),
    };

    let label: Option<Label> = sqlx::query_as(
        r#"
        SELECT * FROM labels
        WHERE repo_owner = $1 AND repo_name = $2 AND name = $3
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(label_name)
    .fetch_optional(pool)
    .await?;

    let label = match label {
        Some(l) => l,
        None => return Ok(false),
    };

    // Insert assignment (ignore if exists)
    sqlx::query(
        r#"
        INSERT INTO issue_labels (issue_id, label_id)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
    "#,
    )
    .bind(issue.id)
    .bind(label.id)
    .execute(pool)
    .await?;

    Ok(true)
}

/// Remove label from issue
pub async fn remove_label_from_issue(
    repo_owner: &str,
    repo_name: &str,
    issue_number: i32,
    label_name: &str,
) -> Result<bool, sqlx::Error> {
    let pool = db::pool();

    let result = sqlx::query(
        r#"
        DELETE FROM issue_labels
        WHERE issue_id = (
            SELECT id FROM issues WHERE repo_owner = $1 AND repo_name = $2 AND number = $3
        )
        AND label_id = (
            SELECT id FROM labels WHERE repo_owner = $1 AND repo_name = $2 AND name = $4
        )
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(issue_number)
    .bind(label_name)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Get labels for an issue
pub async fn get_issue_labels(
    repo_owner: &str,
    repo_name: &str,
    issue_number: i32,
) -> Result<Vec<Label>, sqlx::Error> {
    let pool = db::pool();

    let labels = sqlx::query_as::<_, Label>(
        r#"
        SELECT l.* FROM labels l
        JOIN issue_labels il ON l.id = il.label_id
        JOIN issues i ON il.issue_id = i.id
        WHERE i.repo_owner = $1 AND i.repo_name = $2 AND i.number = $3
        ORDER BY l.name
    "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(issue_number)
    .fetch_all(pool)
    .await?;

    Ok(labels)
}
