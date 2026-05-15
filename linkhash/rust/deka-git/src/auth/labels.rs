/// Seed default labels for a repo (idempotent)
pub async fn seed_labels(repo_owner: &str, repo_name: &str) {
    let labels = [
        ("bug", "e11d48"),
        ("feature", "3b82f6"),
        ("security", "f97316"),
        ("transpiler", "a855f7"),
        ("storefront", "22c55e"),
        ("infra", "9ca3af"),
        ("next-apps", "06b6d4"),
        ("tests", "eab308"),
        ("blocked", "e11d48"),
        ("in-progress", "3b82f6"),
        ("needs-review", "eab308"),
    ];

    let pool = crate::db::pool();
    for (name, color) in labels {
        let _ = sqlx::query(
            "INSERT OR IGNORE INTO labels (repo_owner, repo_name, name, color) VALUES (?, ?, ?, ?)",
        )
        .bind(repo_owner)
        .bind(repo_name)
        .bind(name)
        .bind(color)
        .execute(pool)
        .await;
    }
}
