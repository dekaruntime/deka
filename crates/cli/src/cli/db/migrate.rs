use std::fs;
use std::path::{Path, PathBuf};

use core::Context;
use postgres::{Client, NoTls};
use rusqlite::{Connection, params};
use stdio::{error, log};

use super::config::{DbEngine, read_db_runtime_config};
use super::model::persist_migration_state;

pub(super) fn cmd_migrate(_context: &Context) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let db_dir = cwd.join("db");
    let migrations_dir = db_dir.join("migrations");
    if !migrations_dir.is_dir() {
        error(
            "db migrate",
            "db/migrations directory not found. run `deka db generate <models>` first",
        );
        return;
    }

    let mut migration_files = match collect_migration_files(&migrations_dir) {
        Ok(value) => value,
        Err(message) => {
            error("db migrate", &message);
            return;
        }
    };
    migration_files.sort();
    if migration_files.is_empty() {
        log("db migrate", "no migration files found");
        return;
    }

    let cfg = read_db_runtime_config(&cwd);
    match cfg.engine {
        DbEngine::Postgres => {
            let mut client = match Client::connect(&cfg.location, NoTls) {
                Ok(value) => value,
                Err(err) => {
                    error(
                        "db migrate",
                        &format!("failed to connect to postgres: {}", err),
                    );
                    return;
                }
            };

            if let Err(err) = ensure_migrations_table(&mut client) {
                error(
                    "db migrate",
                    &format!("failed to ensure migration table: {}", err),
                );
                return;
            }

            let applied = match load_applied_migrations(&mut client) {
                Ok(value) => value,
                Err(err) => {
                    error(
                        "db migrate",
                        &format!("failed to read applied migrations: {}", err),
                    );
                    return;
                }
            };

            match apply_migrations(&mut client, &migration_files, &applied, "db migrate") {
                Ok((applied_now, skipped)) => {
                    if let Ok(latest_applied) = load_applied_migrations(&mut client) {
                        if let Err(err) =
                            persist_migration_state(&db_dir, &latest_applied, applied_now, skipped)
                        {
                            error(
                                "db migrate",
                                &format!("migration state write failed: {}", err),
                            );
                        }
                    }
                    log(
                        "db migrate",
                        &format!(
                            "done: engine=postgres applied={}, skipped={}",
                            applied_now, skipped
                        ),
                    );
                }
                Err(message) => error("db migrate", &message),
            }
        }
        DbEngine::Sqlite => {
            let mut conn = match Connection::open(&cfg.location) {
                Ok(value) => value,
                Err(err) => {
                    error(
                        "db migrate",
                        &format!("failed to open sqlite database {}: {}", cfg.location, err),
                    );
                    return;
                }
            };

            if let Err(err) = ensure_migrations_table_sqlite(&mut conn) {
                error(
                    "db migrate",
                    &format!("failed to ensure migration table: {}", err),
                );
                return;
            }

            let applied = match load_applied_migrations_sqlite(&conn) {
                Ok(value) => value,
                Err(err) => {
                    error(
                        "db migrate",
                        &format!("failed to read applied migrations: {}", err),
                    );
                    return;
                }
            };

            match apply_migrations_sqlite(&mut conn, &migration_files, &applied, "db migrate") {
                Ok((applied_now, skipped)) => {
                    if let Ok(latest_applied) = load_applied_migrations_sqlite(&conn) {
                        if let Err(err) =
                            persist_migration_state(&db_dir, &latest_applied, applied_now, skipped)
                        {
                            error(
                                "db migrate",
                                &format!("migration state write failed: {}", err),
                            );
                        }
                    }
                    log(
                        "db migrate",
                        &format!(
                            "done: engine=sqlite applied={}, skipped={}",
                            applied_now, skipped
                        ),
                    );
                }
                Err(message) => error("db migrate", &message),
            }
        }
    }
}

pub(super) fn cmd_info(_context: &Context) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let db_dir = cwd.join("db");
    let state_path = db_dir.join("_state.json");
    let migrations_dir = db_dir.join("migrations");

    if !state_path.is_file() {
        error(
            "db info",
            "db/_state.json not found. run `deka db generate <models>` first",
        );
        return;
    }

    let state_text = match fs::read_to_string(&state_path) {
        Ok(value) => value,
        Err(err) => {
            error(
                "db info",
                &format!("failed to read {}: {}", state_path.display(), err),
            );
            return;
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(&state_text) {
        Ok(value) => value,
        Err(err) => {
            error(
                "db info",
                &format!("failed to parse {}: {}", state_path.display(), err),
            );
            return;
        }
    };

    let model_count = parsed
        .get("model_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let source = parsed
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let generated_at = parsed
        .get("generated_at_unix")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let migration_count = if migrations_dir.is_dir() {
        collect_migration_files(&migrations_dir)
            .map(|v| v.len())
            .unwrap_or(0)
    } else {
        0
    };

    let cfg = read_db_runtime_config(&cwd);
    let engine_name = match cfg.engine {
        DbEngine::Postgres => "postgres",
        DbEngine::Sqlite => "sqlite",
    };

    log("db info", &format!("source: {}", source));
    log("db info", &format!("models: {}", model_count));
    log("db info", &format!("generated_at_unix: {}", generated_at));
    log("db info", &format!("engine: {}", engine_name));
    log("db info", &format!("location: {}", cfg.location));
    log("db info", &format!("migration_files: {}", migration_count));

    let mut applied_count = 0usize;
    let mut pending_count = migration_count;
    match cfg.engine {
        DbEngine::Postgres => {
            if let Ok(mut client) = Client::connect(&cfg.location, NoTls) {
                if ensure_migrations_table(&mut client).is_ok() {
                    if let Ok(applied) = load_applied_migrations(&mut client) {
                        applied_count = applied.len();
                        pending_count = migration_count.saturating_sub(applied_count);
                    }
                }
            }
        }
        DbEngine::Sqlite => {
            if let Ok(mut conn) = Connection::open(&cfg.location) {
                if ensure_migrations_table_sqlite(&mut conn).is_ok() {
                    if let Ok(applied) = load_applied_migrations_sqlite(&conn) {
                        applied_count = applied.len();
                        pending_count = migration_count.saturating_sub(applied_count);
                    }
                }
            }
        }
    }
    log("db info", &format!("applied_migrations: {}", applied_count));
    log("db info", &format!("pending_migrations: {}", pending_count));
}

pub(super) fn cmd_flush(_context: &Context) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let db_dir = cwd.join("db");
    let migrations_dir = db_dir.join("migrations");
    if !migrations_dir.is_dir() {
        error(
            "db flush",
            "db/migrations directory not found. run `deka db generate <models>` first",
        );
        return;
    }

    let mut migration_files = match collect_migration_files(&migrations_dir) {
        Ok(value) => value,
        Err(message) => {
            error("db flush", &message);
            return;
        }
    };
    migration_files.sort();

    let cfg = read_db_runtime_config(&cwd);
    match cfg.engine {
        DbEngine::Postgres => {
            let mut client = match Client::connect(&cfg.location, NoTls) {
                Ok(value) => value,
                Err(err) => {
                    error(
                        "db flush",
                        &format!("failed to connect to postgres: {}", err),
                    );
                    return;
                }
            };

            if let Err(err) =
                client.batch_execute("DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;")
            {
                error("db flush", &format!("failed to reset schema: {}", err));
                return;
            }
            if let Err(err) = ensure_migrations_table(&mut client) {
                error(
                    "db flush",
                    &format!("failed to initialize migration table: {}", err),
                );
                return;
            }

            let none_applied = std::collections::HashSet::new();
            match apply_migrations(&mut client, &migration_files, &none_applied, "db flush") {
                Ok((applied_now, skipped)) => {
                    log(
                        "db flush",
                        &format!(
                            "schema reset complete: engine=postgres applied={}, skipped={}",
                            applied_now, skipped
                        ),
                    );
                }
                Err(message) => error("db flush", &message),
            }
        }
        DbEngine::Sqlite => {
            let mut conn = match Connection::open(&cfg.location) {
                Ok(value) => value,
                Err(err) => {
                    error(
                        "db flush",
                        &format!("failed to open sqlite database {}: {}", cfg.location, err),
                    );
                    return;
                }
            };

            if let Err(err) = reset_sqlite_schema(&conn) {
                error(
                    "db flush",
                    &format!("failed to reset sqlite schema: {}", err),
                );
                return;
            }
            if let Err(err) = ensure_migrations_table_sqlite(&mut conn) {
                error(
                    "db flush",
                    &format!("failed to initialize migration table: {}", err),
                );
                return;
            }

            let none_applied = std::collections::HashSet::new();
            match apply_migrations_sqlite(&mut conn, &migration_files, &none_applied, "db flush") {
                Ok((applied_now, skipped)) => {
                    log(
                        "db flush",
                        &format!(
                            "schema reset complete: engine=sqlite applied={}, skipped={}",
                            applied_now, skipped
                        ),
                    );
                }
                Err(message) => error("db flush", &message),
            }
        }
    }
}

fn collect_migration_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let entries =
        fs::read_dir(dir).map_err(|e| format!("failed to list {}: {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read migration entry: {}", e))?;
        let path = entry.path();
        if path
            .extension()
            .and_then(|v| v.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("sql"))
            .unwrap_or(false)
        {
            files.push(path);
        }
    }
    Ok(files)
}

fn ensure_migrations_table(client: &mut Client) -> Result<(), postgres::Error> {
    client.batch_execute(
        "CREATE TABLE IF NOT EXISTS _deka_migrations (
            version TEXT PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )",
    )
}

fn load_applied_migrations(
    client: &mut Client,
) -> Result<std::collections::HashSet<String>, postgres::Error> {
    let mut out = std::collections::HashSet::new();
    for row in client.query("SELECT version FROM _deka_migrations", &[])? {
        let version: String = row.get(0);
        out.insert(version);
    }
    Ok(out)
}

fn apply_migrations(
    client: &mut Client,
    migration_files: &[PathBuf],
    already_applied: &std::collections::HashSet<String>,
    log_scope: &str,
) -> Result<(usize, usize), String> {
    let mut applied_now = 0usize;
    let mut skipped = 0usize;
    for path in migration_files {
        let version = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("<unknown>")
            .to_string();
        if already_applied.contains(&version) {
            skipped += 1;
            continue;
        }
        let sql = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        if sql.trim().is_empty() {
            skipped += 1;
            continue;
        }

        let mut tx = client
            .transaction()
            .map_err(|err| format!("failed to begin transaction: {}", err))?;
        if let Err(err) = tx.batch_execute(&sql) {
            let _ = tx.rollback();
            return Err(format!("migration {} failed: {}", version, err));
        }
        if let Err(err) = tx.execute(
            "INSERT INTO _deka_migrations (version) VALUES ($1)",
            &[&version],
        ) {
            let _ = tx.rollback();
            return Err(format!("failed to record migration {}: {}", version, err));
        }
        if let Err(err) = tx.commit() {
            return Err(format!("failed to commit migration {}: {}", version, err));
        }
        applied_now += 1;
        log(log_scope, &format!("applied {}", version));
    }
    Ok((applied_now, skipped))
}

fn ensure_migrations_table_sqlite(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _deka_migrations (
            version TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
}

fn load_applied_migrations_sqlite(
    conn: &Connection,
) -> Result<std::collections::HashSet<String>, rusqlite::Error> {
    let mut out = std::collections::HashSet::new();
    let mut stmt = conn.prepare("SELECT version FROM _deka_migrations")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    for version in rows {
        out.insert(version?);
    }
    Ok(out)
}

fn apply_migrations_sqlite(
    conn: &mut Connection,
    migration_files: &[PathBuf],
    already_applied: &std::collections::HashSet<String>,
    log_scope: &str,
) -> Result<(usize, usize), String> {
    let mut applied_now = 0usize;
    let mut skipped = 0usize;
    for path in migration_files {
        let version = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("<unknown>")
            .to_string();
        if already_applied.contains(&version) {
            skipped += 1;
            continue;
        }
        let sql = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        if sql.trim().is_empty() {
            skipped += 1;
            continue;
        }

        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to begin transaction: {}", err))?;
        if let Err(err) = tx.execute_batch(&sql) {
            let _ = tx.rollback();
            return Err(format!("migration {} failed: {}", version, err));
        }
        if let Err(err) = tx.execute(
            "INSERT INTO _deka_migrations (version) VALUES (?1)",
            params![version],
        ) {
            let _ = tx.rollback();
            return Err(format!("failed to record migration {}: {}", version, err));
        }
        if let Err(err) = tx.commit() {
            return Err(format!("failed to commit migration {}: {}", version, err));
        }
        applied_now += 1;
        log(log_scope, &format!("applied {}", version));
    }
    Ok((applied_now, skipped))
}

fn reset_sqlite_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for name in names {
        let escaped = name.replace('"', "\"\"");
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{}\"", escaped))?;
    }
    Ok(())
}
