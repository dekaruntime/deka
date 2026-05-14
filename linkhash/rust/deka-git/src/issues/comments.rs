use crate::{db, redaction};

use super::records::{CreateCommentRequest, IssueComment};
use super::store::get_issue;

pub async fn add_comment(
    repo_owner: &str,
    repo_name: &str,
    number: i64,
    author: &str,
    req: CreateCommentRequest,
) -> Result<Option<IssueComment>, sqlx::Error> {
    let pool = db::pool();

    let issue = get_issue(repo_owner, repo_name, number).await?;
    let issue = match issue {
        Some(i) => i,
        None => return Ok(None),
    };

    let redacted_body = redaction::redact_secret_strings(req.body.trim());
    redaction::log_redactions("issue.comment.create", &redacted_body.redactions);

    let comment = sqlx::query_as::<_, IssueComment>(
        r#"
        INSERT INTO issue_comments (issue_id, body, author)
        VALUES (?, ?, ?)
        RETURNING *
        "#,
    )
    .bind(issue.id)
    .bind(redacted_body.text)
    .bind(author)
    .fetch_one(pool)
    .await?;

    sqlx::query("UPDATE issues SET updated_at = datetime('now') WHERE id = ?")
        .bind(issue.id)
        .execute(pool)
        .await?;

    Ok(Some(comment))
}

pub async fn list_comments(
    repo_owner: &str,
    repo_name: &str,
    number: i64,
) -> Result<Vec<IssueComment>, sqlx::Error> {
    let pool = db::pool();
    sqlx::query_as::<_, IssueComment>(
        r#"
        SELECT c.* FROM issue_comments c
        JOIN issues i ON c.issue_id = i.id
        WHERE i.repo_owner = ? AND i.repo_name = ? AND i.number = ?
        ORDER BY c.created_at ASC
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_all(pool)
    .await
}
