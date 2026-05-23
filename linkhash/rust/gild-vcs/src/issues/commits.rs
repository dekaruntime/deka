use crate::db;

use super::records::CommitRef;

pub async fn add_commit_ref(
    issue_id: i64,
    commit_hash: &str,
    repo: &str,
) -> Result<CommitRef, sqlx::Error> {
    let pool = db::pool();
    sqlx::query_as::<_, CommitRef>(
        "INSERT INTO commit_refs (issue_id, commit_hash, repo) VALUES (?, ?, ?) RETURNING *",
    )
    .bind(issue_id)
    .bind(commit_hash)
    .bind(repo)
    .fetch_one(pool)
    .await
}

pub async fn get_commit_refs(issue_id: i64) -> Result<Vec<CommitRef>, sqlx::Error> {
    let pool = db::pool();
    sqlx::query_as::<_, CommitRef>(
        "SELECT * FROM commit_refs WHERE issue_id = ? ORDER BY created_at ASC",
    )
    .bind(issue_id)
    .fetch_all(pool)
    .await
}
