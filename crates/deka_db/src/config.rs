use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbEngine {
    Postgres,
    Sqlite,
}

#[derive(Debug, Clone)]
pub struct DbRuntimeConfig {
    pub engine: DbEngine,
    pub location: String,
}

fn load_deka_json(cwd: &Path) -> Option<serde_json::Value> {
    let path = cwd.join("deka.json");
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[doc(hidden)]
pub fn read_db_runtime_config(cwd: &Path) -> DbRuntimeConfig {
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
            let location = location_raw.unwrap_or_else(postgres_connection_string);
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

    let location = postgres_connection_string();
    DbRuntimeConfig {
        engine: DbEngine::Postgres,
        location,
    }
}

fn postgres_connection_string() -> String {
    const DEFAULT_HOST: &str = "127.0.0.1";
    const DEFAULT_PORT: &str = "55432";
    const DEFAULT_NAME: &str = "linkhash_registry";
    const DEFAULT_USER: &str = "postgres";
    const DEFAULT_PASSWORD: &str = "postgres";
    format!(
        "host={} port={} dbname={} user={} password={}",
        DEFAULT_HOST, DEFAULT_PORT, DEFAULT_NAME, DEFAULT_USER, DEFAULT_PASSWORD
    )
}
