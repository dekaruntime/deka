use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DbEngine {
    Postgres,
    Sqlite,
}

#[derive(Debug, Clone)]
pub(super) struct DbRuntimeConfig {
    pub(super) engine: DbEngine,
    pub(super) location: String,
}

fn load_deka_json(cwd: &Path) -> Option<serde_json::Value> {
    let path = cwd.join("deka.json");
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub(super) fn read_db_runtime_config(cwd: &Path) -> DbRuntimeConfig {
    let json = load_deka_json(cwd);
    let db = json
        .as_ref()
        .and_then(|v| v.get("db"))
        .and_then(|v| v.as_object());

    let engine_raw = db
        .and_then(|obj| obj.get("engine"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase());
    let location_raw = db
        .and_then(|obj| obj.get("location"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    match engine_raw.as_deref() {
        Some("sqlite") => {
            let location =
                location_raw.unwrap_or_else(|| cwd.join("deka.sqlite").display().to_string());
            return DbRuntimeConfig {
                engine: DbEngine::Sqlite,
                location,
            };
        }
        Some("postgres") | Some("pg") => {
            let location = location_raw
                .or_else(|| std::env::var("DATABASE_URL").ok())
                .unwrap_or_else(postgres_connection_string);
            return DbRuntimeConfig {
                engine: DbEngine::Postgres,
                location,
            };
        }
        _ => {}
    }

    if let Some(location) = location_raw {
        if location.ends_with(".sqlite") {
            let resolved = if Path::new(&location).is_absolute() {
                location
            } else {
                cwd.join(location).display().to_string()
            };
            return DbRuntimeConfig {
                engine: DbEngine::Sqlite,
                location: resolved,
            };
        }
        return DbRuntimeConfig {
            engine: DbEngine::Postgres,
            location,
        };
    }

    let location = std::env::var("DATABASE_URL").unwrap_or_else(|_| postgres_connection_string());
    DbRuntimeConfig {
        engine: DbEngine::Postgres,
        location,
    }
}

fn postgres_connection_string() -> String {
    let host = std::env::var("DB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("DB_PORT").unwrap_or_else(|_| "55432".to_string());
    let name = std::env::var("DB_NAME").unwrap_or_else(|_| "linkhash_registry".to_string());
    let user = std::env::var("DB_USER").unwrap_or_else(|_| "postgres".to_string());
    let pass = std::env::var("DB_PASSWORD").unwrap_or_else(|_| "postgres".to_string());
    format!(
        "host={} port={} dbname={} user={} password={}",
        host, port, name, user, pass
    )
}
