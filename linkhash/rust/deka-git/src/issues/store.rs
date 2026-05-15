use crate::{db, redaction};

use super::records::{CreateIssueRequest, Issue, ListIssuesQuery, UpdateIssueRequest};

async fn get_next_issue_number(repo_owner: &str, repo_name: &str) -> Result<i64, sqlx::Error> {
    let pool = db::pool();

    // SQLite doesn't have ON CONFLICT ... RETURNING, so we do it in two steps.
    sqlx::query(
        r#"
        INSERT INTO issue_sequences (repo_owner, repo_name, next_number)
        VALUES (?, ?, 2)
        ON CONFLICT (repo_owner, repo_name)
        DO UPDATE SET next_number = issue_sequences.next_number + 1
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .execute(pool)
    .await?;

    let result: (i64,) = sqlx::query_as(
        "SELECT next_number - 1 FROM issue_sequences WHERE repo_owner = ? AND repo_name = ?",
    )
    .bind(repo_owner)
    .bind(repo_name)
    .fetch_one(pool)
    .await?;

    Ok(result.0)
}

pub async fn create_issue(
    repo_owner: &str,
    repo_name: &str,
    author: &str,
    req: CreateIssueRequest,
) -> Result<Issue, sqlx::Error> {
    let pool = db::pool();
    let number = get_next_issue_number(repo_owner, repo_name).await?;
    let redacted_title = redaction::redact_secret_strings(&req.title);
    redaction::log_redactions("issue.title.create", &redacted_title.redactions);
    redaction::alert_redactions(
        redaction::RedactionAlertContext {
            repo_owner,
            repo_name,
            location: "issue.title.create",
            subject: Some(format!("#{}", number)),
            caller: author,
        },
        &redacted_title.redactions,
    )
    .await;
    let (redacted_body, body_redactions) = redaction::redact_optional_secret_strings(req.body);
    redaction::log_redactions("issue.body.create", &body_redactions);
    redaction::alert_redactions(
        redaction::RedactionAlertContext {
            repo_owner,
            repo_name,
            location: "issue.body.create",
            subject: Some(format!("#{}", number)),
            caller: author,
        },
        &body_redactions,
    )
    .await;

    let issue = sqlx::query_as::<_, Issue>(
        r#"
        INSERT INTO issues (repo_owner, repo_name, number, title, body, author, assignee, priority, repo)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING *
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .bind(redacted_title.text)
    .bind(redacted_body)
    .bind(author)
    .bind(&req.assignee)
    .bind(req.priority.as_deref().unwrap_or("p2"))
    .bind(&req.repo)
    .fetch_one(pool)
    .await?;

    Ok(issue)
}

pub async fn list_issues(
    repo_owner: &str,
    repo_name: &str,
    query: &ListIssuesQuery,
) -> Result<Vec<Issue>, sqlx::Error> {
    let pool = db::pool();

    let state = match query.state.as_deref() {
        Some("closed") | Some("done") => "closed",
        Some("in-progress") => "in-progress",
        Some("all") => "%",
        _ => "open",
    };

    let mut sql = String::from(
        "SELECT * FROM issues WHERE repo_owner = ?1 AND repo_name = ?2 AND state LIKE ?3",
    );
    if query.assignee.is_some() {
        sql.push_str(" AND assignee = ?4");
    }
    if query.priority.is_some() {
        sql.push_str(" AND priority = ?5");
    }
    if query.repo.is_some() {
        sql.push_str(" AND repo = ?6");
    }
    sql.push_str(" ORDER BY number DESC");

    let mut q = sqlx::query_as::<_, Issue>(&sql)
        .bind(repo_owner)
        .bind(repo_name)
        .bind(state);

    if let Some(ref assignee) = query.assignee {
        q = q.bind(assignee);
    }
    if let Some(ref priority) = query.priority {
        q = q.bind(priority);
    }
    if let Some(ref repo) = query.repo {
        q = q.bind(repo);
    }

    q.fetch_all(pool).await
}

pub async fn get_issue(
    repo_owner: &str,
    repo_name: &str,
    number: i64,
) -> Result<Option<Issue>, sqlx::Error> {
    let pool = db::pool();
    sqlx::query_as::<_, Issue>(
        "SELECT * FROM issues WHERE repo_owner = ? AND repo_name = ? AND number = ?",
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_optional(pool)
    .await
}

pub async fn update_issue(
    repo_owner: &str,
    repo_name: &str,
    number: i64,
    caller: &str,
    req: UpdateIssueRequest,
) -> Result<Option<Issue>, sqlx::Error> {
    let pool = db::pool();

    let existing = get_issue(repo_owner, repo_name, number).await?;
    let existing = match existing {
        Some(i) => i,
        None => return Ok(None),
    };

    let new_title_raw = req.title.unwrap_or(existing.title);
    let new_body_raw = req.body.or(existing.body);
    let new_title = redaction::redact_secret_strings(&new_title_raw);
    redaction::log_redactions("issue.title.update", &new_title.redactions);
    redaction::alert_redactions(
        redaction::RedactionAlertContext {
            repo_owner,
            repo_name,
            location: "issue.title.update",
            subject: Some(format!("#{}", number)),
            caller,
        },
        &new_title.redactions,
    )
    .await;
    let (new_body, body_redactions) = redaction::redact_optional_secret_strings(new_body_raw);
    redaction::log_redactions("issue.body.update", &body_redactions);
    redaction::alert_redactions(
        redaction::RedactionAlertContext {
            repo_owner,
            repo_name,
            location: "issue.body.update",
            subject: Some(format!("#{}", number)),
            caller,
        },
        &body_redactions,
    )
    .await;
    let new_state = req.state.unwrap_or(existing.state.clone());
    let new_assignee = req.assignee.or(existing.assignee);
    let new_priority = req.priority.or(existing.priority);

    let closed_at = if (new_state == "closed" || new_state == "done")
        && existing.state != "closed"
        && existing.state != "done"
    {
        Some(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string())
    } else if new_state == "open" || new_state == "in-progress" {
        None
    } else {
        existing.closed_at
    };

    let issue = sqlx::query_as::<_, Issue>(
        r#"
        UPDATE issues
        SET title = ?, body = ?, state = ?, assignee = ?, priority = ?,
            closed_at = ?, updated_at = datetime('now')
        WHERE repo_owner = ? AND repo_name = ? AND number = ?
        RETURNING *
        "#,
    )
    .bind(new_title.text)
    .bind(new_body)
    .bind(&new_state)
    .bind(&new_assignee)
    .bind(&new_priority)
    .bind(&closed_at)
    .bind(repo_owner)
    .bind(repo_name)
    .bind(number)
    .fetch_optional(pool)
    .await?;

    Ok(issue)
}
