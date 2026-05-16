use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::sync::OnceLock;

static DB_POOL: OnceLock<SqlitePool> = OnceLock::new();

pub async fn init(db_path: &str) -> anyhow::Result<()> {
    tracing::info!("Opening SQLite database at {}", db_path);

    let opts = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    run_migrations(&pool).await?;

    DB_POOL.set(pool).expect("Database pool already initialized");

    tracing::info!("Database initialized");
    Ok(())
}

pub fn pool() -> &'static SqlitePool {
    DB_POOL.get().expect("Database not initialized")
}

async fn run_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            email TEXT NOT NULL,
            account_type TEXT NOT NULL CHECK (account_type IN ('agent', 'user', 'customer', 'system')),
            display_name TEXT,
            created_at TEXT DEFAULT (datetime('now')),
            updated_at TEXT DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS account_ssh_keys (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            key_name TEXT NOT NULL,
            public_key TEXT NOT NULL UNIQUE,
            fingerprint TEXT NOT NULL UNIQUE,
            created_at TEXT DEFAULT (datetime('now')),
            revoked INTEGER DEFAULT 0,
            FOREIGN KEY (account_id) REFERENCES accounts(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS access_tokens (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            key_hash TEXT NOT NULL UNIQUE,
            key_type TEXT NOT NULL CHECK (key_type IN ('system', 'agent', 'user')),
            owner TEXT NOT NULL,
            scopes TEXT NOT NULL DEFAULT '["repo:read"]',
            repos TEXT NOT NULL DEFAULT '["*"]',
            created_at TEXT DEFAULT (datetime('now')),
            expires_at TEXT,
            last_used_at TEXT,
            revoked INTEGER DEFAULT 0
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS repo_acl (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account TEXT NOT NULL,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            access TEXT NOT NULL CHECK (access IN ('read', 'write')),
            granted_by TEXT,
            created_at TEXT DEFAULT (datetime('now')),
            UNIQUE(account, repo_owner, repo_name)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS secret_acl (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account TEXT NOT NULL,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            secret_pattern TEXT NOT NULL,
            granted_by TEXT,
            created_at TEXT DEFAULT (datetime('now')),
            UNIQUE(account, repo_owner, repo_name, secret_pattern)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS webhook_subscriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            url TEXT NOT NULL,
            events TEXT NOT NULL DEFAULT '["push"]',
            created_by TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now')),
            active INTEGER DEFAULT 1,
            UNIQUE(repo_owner, repo_name, url)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS deploy_watchers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            watcher_url TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1,
            created_by TEXT NOT NULL DEFAULT 'system',
            created_at TEXT DEFAULT (datetime('now')),
            updated_at TEXT DEFAULT (datetime('now')),
            last_branch TEXT,
            last_sha TEXT,
            last_delivery_status TEXT,
            last_delivery_code INTEGER,
            last_delivery_error TEXT,
            last_delivery_at TEXT,
            UNIQUE(repo_owner, repo_name)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token_id INTEGER,
            key_type TEXT NOT NULL,
            owner TEXT NOT NULL,
            action TEXT NOT NULL,
            repo TEXT,
            ref_name TEXT,
            detail TEXT,
            ip TEXT,
            timestamp TEXT DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS issues (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            number INTEGER NOT NULL,
            title TEXT NOT NULL,
            body TEXT,
            state TEXT NOT NULL DEFAULT 'open',
            author TEXT NOT NULL,
            assignee TEXT,
            priority TEXT DEFAULT 'p2' CHECK (priority IN ('p0', 'p1', 'p2', 'p3')),
            repo TEXT,
            created_at TEXT DEFAULT (datetime('now')),
            updated_at TEXT DEFAULT (datetime('now')),
            closed_at TEXT,
            UNIQUE(repo_owner, repo_name, number)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS issue_comments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            issue_id INTEGER NOT NULL,
            body TEXT NOT NULL,
            author TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now')),
            updated_at TEXT,
            FOREIGN KEY (issue_id) REFERENCES issues(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS labels (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            name TEXT NOT NULL,
            color TEXT NOT NULL DEFAULT '6e7681',
            description TEXT,
            UNIQUE(repo_owner, repo_name, name)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS issue_labels (
            issue_id INTEGER NOT NULL,
            label_id INTEGER NOT NULL,
            PRIMARY KEY (issue_id, label_id),
            FOREIGN KEY (issue_id) REFERENCES issues(id),
            FOREIGN KEY (label_id) REFERENCES labels(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS issue_sequences (
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            next_number INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY (repo_owner, repo_name)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pull_requests (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            number INTEGER NOT NULL,
            title TEXT NOT NULL,
            body TEXT,
            state TEXT NOT NULL DEFAULT 'open',
            author TEXT NOT NULL,
            source_ref TEXT NOT NULL,
            target_ref TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now')),
            updated_at TEXT DEFAULT (datetime('now')),
            closed_at TEXT,
            UNIQUE(repo_owner, repo_name, number)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pull_comments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            pull_id INTEGER NOT NULL,
            body TEXT NOT NULL,
            author TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now')),
            updated_at TEXT,
            FOREIGN KEY (pull_id) REFERENCES pull_requests(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pull_sequences (
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            next_number INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY (repo_owner, repo_name)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS commit_refs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            issue_id INTEGER NOT NULL,
            commit_hash TEXT NOT NULL,
            repo TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now')),
            FOREIGN KEY (issue_id) REFERENCES issues(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Package registry tables
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS package_releases (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            package_name TEXT NOT NULL,
            version TEXT NOT NULL,
            owner TEXT NOT NULL,
            repo TEXT NOT NULL,
            git_ref TEXT NOT NULL,
            description TEXT,
            manifest TEXT,
            api_snapshot TEXT,
            api_change_kind TEXT,
            required_bump TEXT,
            capability_metadata TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(package_name, version)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS package_sequences (
            package_name TEXT PRIMARY KEY,
            next_number INTEGER NOT NULL DEFAULT 1
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Repo visibility
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS repo_visibility (
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            visibility TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('public', 'private')),
            PRIMARY KEY (repo_owner, repo_name)
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Create index on audit_log for common queries
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_audit_log_action ON audit_log(action)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_audit_log_owner ON audit_log(owner)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_repo_acl_account ON repo_acl(account)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_secret_acl_account ON secret_acl(account)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_webhook_subscriptions_repo ON webhook_subscriptions(repo_owner, repo_name)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_deploy_watchers_active ON deploy_watchers(repo_owner, repo_name, active)",
    )
    .execute(pool)
    .await?;

    tracing::info!("Database migrations complete");
    Ok(())
}
