use crate::db;

use super::records::Label;
use super::store::get_issue;

pub async fn create_label(
    repo_owner: &str,
    repo_name: &str,
    name: &str,
    color: &str,
    description: Option<&str>,
) -> Result<Label, sqlx::Error> {
    let pool = db::pool();
    sqlx::query_as::<_, Label>(
        r#"
        INSERT INTO labels (repo_owner, repo_name, name, color, description)
        VALUES (?, ?, ?, ?, ?)
        RETURNING *
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(name)
    .bind(color)
    .bind(description)
    .fetch_one(pool)
    .await
}

pub async fn list_labels(repo_owner: &str, repo_name: &str) -> Result<Vec<Label>, sqlx::Error> {
    let pool = db::pool();
    sqlx::query_as::<_, Label>(
        "SELECT * FROM labels WHERE repo_owner = ? AND repo_name = ? ORDER BY name",
    )
    .bind(repo_owner)
    .bind(repo_name)
    .fetch_all(pool)
    .await
}

pub async fn add_label_to_issue(
    repo_owner: &str,
    repo_name: &str,
    issue_number: i64,
    label_name: &str,
) -> Result<bool, sqlx::Error> {
    let pool = db::pool();

    let issue = get_issue(repo_owner, repo_name, issue_number).await?;
    let issue = match issue {
        Some(i) => i,
        None => return Ok(false),
    };

    let label: Option<Label> =
        sqlx::query_as("SELECT * FROM labels WHERE repo_owner = ? AND repo_name = ? AND name = ?")
            .bind(repo_owner)
            .bind(repo_name)
            .bind(label_name)
            .fetch_optional(pool)
            .await?;

    let label = match label {
        Some(l) => l,
        None => return Ok(false),
    };

    sqlx::query("INSERT OR IGNORE INTO issue_labels (issue_id, label_id) VALUES (?, ?)")
        .bind(issue.id)
        .bind(label.id)
        .execute(pool)
        .await?;

    Ok(true)
}

pub async fn remove_label_from_issue(
    repo_owner: &str,
    repo_name: &str,
    issue_number: i64,
    label_name: &str,
) -> Result<bool, sqlx::Error> {
    let pool = db::pool();

    let result = sqlx::query(
        r#"
        DELETE FROM issue_labels
        WHERE issue_id = (
            SELECT id FROM issues WHERE repo_owner = ?1 AND repo_name = ?2 AND number = ?3
        )
        AND label_id = (
            SELECT id FROM labels WHERE repo_owner = ?1 AND repo_name = ?2 AND name = ?4
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

pub async fn get_issue_labels(
    repo_owner: &str,
    repo_name: &str,
    issue_number: i64,
) -> Result<Vec<Label>, sqlx::Error> {
    let pool = db::pool();
    sqlx::query_as::<_, Label>(
        r#"
        SELECT l.* FROM labels l
        JOIN issue_labels il ON l.id = il.label_id
        JOIN issues i ON il.issue_id = i.id
        WHERE i.repo_owner = ? AND i.repo_name = ? AND i.number = ?
        ORDER BY l.name
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(issue_number)
    .fetch_all(pool)
    .await
}
