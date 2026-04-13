use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::OnceLock;

static DB_POOL: OnceLock<PgPool> = OnceLock::new();

pub async fn init_with_url(database_url: &str) -> anyhow::Result<()> {
    tracing::info!("Connecting to database...");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;

    verify_schema_ready(&pool).await?;

    DB_POOL
        .set(pool)
        .expect("Database pool already initialized");

    tracing::info!("Database connected and schema verified");
    Ok(())
}

pub fn pool() -> &'static PgPool {
    DB_POOL.get().expect("Database not initialized")
}

pub async fn ensure_bootstrap_identity(
    username: &str,
    raw_token: &str,
) -> anyhow::Result<()> {
    let pool = pool();

    let user_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO users (username)
        VALUES ($1)
        ON CONFLICT (username) DO UPDATE SET username = EXCLUDED.username
        RETURNING id
        "#,
    )
    .bind(username)
    .fetch_one(pool)
    .await?;

    let token_hash = sha256_hex(raw_token);
    let created = sqlx::query(
        r#"
        INSERT INTO user_tokens (user_id, token_name, token_hash)
        VALUES ($1, $2, $3)
        ON CONFLICT (token_hash) DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind("bootstrap")
    .bind(token_hash)
    .execute(pool)
    .await?;

    if created.rows_affected() > 0 {
        tracing::warn!(
            "Created bootstrap token for {}. Rotate token after first login.",
            username
        );
    }

    Ok(())
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

async fn verify_schema_ready(pool: &PgPool) -> anyhow::Result<()> {
    let required_tables = [
        "users",
        "user_tokens",
        "user_ssh_keys",
        "package_releases",
        "issues",
        "issue_comments",
        "labels",
        "issue_labels",
        "issue_sequences",
        "pull_requests",
        "pull_comments",
        "pull_sequences",
    ];

    for table in required_tables {
        require_table(pool, table).await?;
    }

    require_columns(
        pool,
        "users",
        &[
            "id",
            "username",
            "email",
            "password_hash",
            "status",
            "email_verified_at",
            "display_name",
            "created_at",
        ],
    )
    .await?;
    require_columns(
        pool,
        "user_tokens",
        &[
            "id",
            "user_id",
            "token_name",
            "token_hash",
            "created_at",
            "last_used_at",
            "expires_at",
            "revoked_at",
        ],
    )
    .await?;
    require_columns(
        pool,
        "package_releases",
        &[
            "id",
            "package_name",
            "version",
            "owner",
            "repo",
            "git_ref",
            "description",
            "manifest",
            "api_snapshot",
            "api_change_kind",
            "required_bump",
            "capability_metadata",
            "created_at",
        ],
    )
    .await?;
    require_columns(
        pool,
        "issues",
        &[
            "id",
            "repo_owner",
            "repo_name",
            "number",
            "title",
            "body",
            "state",
            "author",
            "created_at",
            "updated_at",
            "closed_at",
        ],
    )
    .await?;
    require_columns(
        pool,
        "pull_requests",
        &[
            "id",
            "repo_owner",
            "repo_name",
            "number",
            "title",
            "body",
            "state",
            "author",
            "source_ref",
            "target_ref",
            "created_at",
            "updated_at",
            "closed_at",
        ],
    )
    .await?;

    Ok(())
}

async fn require_table(pool: &PgPool, table: &str) -> anyhow::Result<()> {
    let present: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
        .bind(format!("public.{table}"))
        .fetch_one(pool)
        .await?;
    if present.is_none() {
        anyhow::bail!(
            "missing required table `{}`. run migrations from `linkhash/phpx` (for example: `deka db migrate`)",
            table
        );
    }
    Ok(())
}

async fn require_columns(pool: &PgPool, table: &str, columns: &[&str]) -> anyhow::Result<()> {
    let existing: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT column_name
        FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = $1
        "#,
    )
    .bind(table)
    .fetch_all(pool)
    .await?;

    for column in columns {
        if !existing.iter().any(|current| current == column) {
            anyhow::bail!(
                "table `{}` is missing required column `{}`. run migrations from `linkhash/phpx` (for example: `deka db migrate`)",
                table,
                column
            );
        }
    }
    Ok(())
}
