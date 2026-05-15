/// Get visibility for a repo. Defaults to "private" if not set.
pub async fn get_repo_visibility(owner: &str, name: &str) -> String {
    let pool = crate::db::pool();
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT visibility FROM repo_visibility WHERE repo_owner = ? AND repo_name = ?",
    )
    .bind(owner)
    .bind(name)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);

    row.map(|r| r.0).unwrap_or_else(|| "private".to_string())
}

/// Set visibility for a repo (upsert).
pub async fn set_repo_visibility(owner: &str, name: &str, visibility: &str) -> anyhow::Result<()> {
    let pool = crate::db::pool();
    sqlx::query(
        r#"
        INSERT INTO repo_visibility (repo_owner, repo_name, visibility)
        VALUES (?, ?, ?)
        ON CONFLICT(repo_owner, repo_name) DO UPDATE SET visibility = excluded.visibility
        "#,
    )
    .bind(owner)
    .bind(name)
    .bind(visibility)
    .execute(pool)
    .await?;
    Ok(())
}

/// Check if a repo is public.
pub async fn is_repo_public(owner: &str, name: &str) -> bool {
    get_repo_visibility(owner, name).await == "public"
}
