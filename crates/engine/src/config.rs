use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize)]
pub struct RuntimeConfig {
    pub code_cache: Option<CodeCacheConfig>,
    pub introspect: Option<IntrospectConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServeMode {
    Static,
    #[serde(alias = "ds")]
    Php,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServeKind {
    Static,
    Worker,
}

#[derive(Debug, Default, Deserialize)]
pub struct ServeConfig {
    pub mode: Option<ServeMode>,
    pub entry: Option<String>,
    pub directory_listing: Option<bool>,
    /// `"static"` or `"worker"`. Omitted → Worker iff `api/` routes or `server:defer` islands exist.
    pub kind: Option<ServeKind>,
    /// Canonical trailing slash. Default: no trailing slash except `/`.
    #[serde(default, alias = "trailing_slash")]
    pub trailing_slash: Option<bool>,
}

impl ServeConfig {
    pub fn load(directory: &std::path::Path) -> Self {
        let deka_json_path = directory.join("deka.json");
        if let Some(config) = load_serve_from_deka_json(&deka_json_path) {
            return config;
        }

        // Backward compatibility: keep reading serve.json if present.
        let legacy_path = directory.join("serve.json");
        load_legacy_serve_json(&legacy_path).unwrap_or_default()
    }
}

fn load_serve_from_deka_json(path: &std::path::Path) -> Option<ServeConfig> {
    if !path.exists() {
        return None;
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) => {
            tracing::warn!("Failed to read {}: {}", path.display(), err);
            return None;
        }
    };

    let root: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!("Failed to parse {}: {}", path.display(), err);
            return None;
        }
    };

    if let Some(serve) = root.get("serve") {
        match serde_json::from_value::<ServeConfig>(serve.clone()) {
            Ok(config) => return Some(config),
            Err(err) => {
                tracing::warn!("Failed to parse {}.serve: {}", path.display(), err);
                return None;
            }
        }
    }

    // Optional convenience: allow top-level serve keys in deka.json.
    match serde_json::from_value::<ServeConfig>(root) {
        Ok(config)
            if config.entry.is_some()
                || config.mode.is_some()
                || config.directory_listing.is_some() =>
        {
            Some(config)
        }
        _ => None,
    }
}

fn load_legacy_serve_json(path: &std::path::Path) -> Option<ServeConfig> {
    if !path.exists() {
        return None;
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) => {
            tracing::warn!("Failed to read {}: {}", path.display(), err);
            return None;
        }
    };

    match serde_json::from_str::<ServeConfig>(&contents) {
        Ok(config) => Some(config),
        Err(err) => {
            tracing::warn!("Failed to parse {}: {}", path.display(), err);
            None
        }
    }
}

pub struct ResolvedHandler {
    pub path: PathBuf,
    pub mode: ServeMode,
    pub config: ServeConfig,
}

/// Resolve a handler path, detecting directories and index files
pub fn resolve_handler_path(path: &str) -> Result<ResolvedHandler, String> {
    let path = std::path::Path::new(path);
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let cwd = std::env::current_dir().map_err(|e| format!("Failed to get cwd: {}", e))?;
        cwd.join(path)
    };

    // Canonicalize if it exists
    let abs_path = if abs_path.exists() {
        abs_path.canonicalize().unwrap_or(abs_path)
    } else {
        abs_path
    };

    // Check if it's a directory
    let is_dir = abs_path.is_dir();

    let (handler_dir, serve_config) = if is_dir {
        let config = ServeConfig::load(&abs_path);
        load_database_config(&abs_path);
        (abs_path.clone(), config)
    } else if let Some(parent) = abs_path.parent() {
        let config = ServeConfig::load(parent);
        load_database_config(parent);
        (parent.to_path_buf(), config)
    } else {
        (PathBuf::from("."), ServeConfig::default())
    };

    // Explicit file input always wins over config entry defaults.
    if !is_dir {
        let mode = serve_config
            .mode
            .clone()
            .unwrap_or_else(|| detect_mode(&abs_path));
        return Ok(ResolvedHandler {
            path: abs_path,
            mode,
            config: serve_config,
        });
    }

    if let Some(ref entry) = serve_config.entry {
        let entry_path = if std::path::Path::new(entry).is_absolute() {
            PathBuf::from(entry)
        } else {
            handler_dir.join(entry)
        };

        if !entry_path.exists() {
            return Err(format!("Entry file not found: {}", entry_path.display()));
        }

        let mode = serve_config
            .mode
            .clone()
            .unwrap_or_else(|| detect_mode(&entry_path));
        return Ok(ResolvedHandler {
            path: entry_path,
            mode,
            config: serve_config,
        });
    }

    if runtime_core::framework::is_source_app_router_project(&handler_dir) {
        let entry_path = runtime_core::framework::write_app_router_entry(&handler_dir)?;
        return Ok(ResolvedHandler {
            path: entry_path,
            mode: serve_config.mode.clone().unwrap_or(ServeMode::Php),
            config: serve_config,
        });
    }

    // JS is a WinterTC worker; HTML stays static.
    let index_files = ["index.ds", "index.dsx", "index.js", "index.mjs", "index.cjs", "index.html"];

    for index_file in &index_files {
        let index_path = abs_path.join(index_file);
        if index_path.exists() {
            let mode = serve_config
                .mode
                .clone()
                .unwrap_or_else(|| detect_mode(&index_path));
            return Ok(ResolvedHandler {
                path: index_path,
                mode,
                config: serve_config,
            });
        }
    }

    // No index file found - use static directory serving
    Ok(ResolvedHandler {
        path: abs_path,
        mode: serve_config.mode.clone().unwrap_or(ServeMode::Static),
        config: serve_config,
    })
}

fn detect_mode(path: &std::path::Path) -> ServeMode {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "ds" | "dsx" | "js" | "mjs" | "cjs" => ServeMode::Php,
            "html" | "htm" => ServeMode::Static,
            _ => ServeMode::Static,
        }
    } else {
        ServeMode::Static
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CodeCacheConfig {
    pub enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct IntrospectConfig {
    pub retention_days: Option<u64>,
    pub db_path: Option<String>,
    pub profiling: Option<bool>,
}

impl RuntimeConfig {
    pub fn load() -> Self {
        let path = match Self::find_config_path() {
            Some(path) => path,
            None => return Self::default(),
        };

        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(_) => return Self::default(),
        };

        match toml::from_str::<RuntimeConfig>(&contents) {
            Ok(config) => config,
            Err(err) => {
                tracing::warn!("Failed to parse {}: {}", path.display(), err);
                Self::default()
            }
        }
    }

    pub fn code_cache_enabled(&self) -> Option<bool> {
        self.code_cache.as_ref()?.enabled
    }

    pub fn introspect_retention_days(&self) -> u64 {
        self.introspect
            .as_ref()
            .and_then(|config| config.retention_days)
            .unwrap_or(7)
    }

    pub fn introspect_db_path(&self) -> Option<PathBuf> {
        if let Some(path) = self
            .introspect
            .as_ref()
            .and_then(|config| config.db_path.as_ref())
        {
            return Some(expand_home_path(path));
        }

        default_introspect_db_path()
    }

    pub fn introspect_profiling_enabled(&self) -> bool {
        self.introspect
            .as_ref()
            .and_then(|config| config.profiling)
            .unwrap_or(true)
    }

    fn find_config_path() -> Option<PathBuf> {
        let mut candidates = Vec::new();

        if let Ok(path) = std::env::var("DEKA_RUNTIME_CONFIG") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
            tracing::warn!(
                "DEKA_RUNTIME_CONFIG set but file not found: {}",
                path.display()
            );
            candidates.push(path);
        }

        candidates.push(PathBuf::from("config.toml"));
        candidates.push(PathBuf::from("runtime.toml"));
        candidates.push(PathBuf::from("deka-runtime.toml"));

        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        {
            candidates.push(path.join("deka").join("config.toml"));
            candidates.push(path.join("deka").join("runtime.toml"));
            candidates.push(path.join("deka").join("deka-runtime.toml"));
        }

        candidates.push(PathBuf::from("/etc/deka/config.toml"));
        candidates.push(PathBuf::from("/etc/deka/runtime.toml"));
        candidates.push(PathBuf::from("/etc/deka/deka-runtime.toml"));

        candidates.into_iter().find(|path| path.exists())
    }
}

fn default_introspect_db_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".deka").join("introspect.db"))
}

fn expand_home_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{}_{}", prefix, nonce));
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn app_directory_respects_serve_entry_for_directory_input() {
        let dir = temp_dir("deka_engine_app_over_entry");
        let app_dir = dir.join("app");
        fs::create_dir_all(&app_dir).expect("mkdir app");
        fs::write(app_dir.join("page.ds"), "export function page() { return 'ok'; }")
            .expect("write page");
        fs::write(dir.join("main.ds"), "export function main() { return 'main'; }")
            .expect("write configured");
        fs::write(dir.join("serve.json"), r#"{"entry":"main.ds"}"#).expect("write config");

        let resolved = resolve_handler_path(dir.to_str().expect("path")).expect("resolve");
        let resolved_canon = resolved.path.canonicalize().expect("resolved canonicalize");
        let configured_canon = dir
            .join("main.ds")
            .canonicalize()
            .expect("configured canonicalize");
        assert_eq!(resolved_canon, configured_canon);
    }

    #[test]
    fn directory_without_index_or_app_router_falls_back_to_static() {
        let dir = temp_dir("deka_engine_static_fallback");
        let app_dir = dir.join("app");
        fs::create_dir_all(&app_dir).expect("mkdir app");
        fs::write(app_dir.join("notes.txt"), "not a route").expect("write app file");

        let resolved = resolve_handler_path(dir.to_str().expect("path")).expect("resolve");
        assert!(resolved.path.is_dir());
        assert!(matches!(resolved.mode, ServeMode::Static));
    }
}

/// Load database configuration from deka.json and set environment variables.
/// Called during handler initialization so the neo4j/redis modules pick up config.
///
/// Example deka.json:
/// ```json
/// {
///   "neo4j": { "uri": "bolt://localhost:7687", "user": "neo4j", "password": "secret", "db": "neo4j" },
///   "redis": { "url": "redis://localhost:6379" }
/// }
/// ```
pub fn load_database_config(directory: &std::path::Path) {
    let deka_json = directory.join("deka.json");
    if !deka_json.exists() {
        return;
    }

    let contents = match std::fs::read_to_string(&deka_json) {
        Ok(c) => c,
        Err(_) => return,
    };

    let root: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return,
    };

    // SAFETY: called once during single-threaded init before handler threads start.
    unsafe {
        // Neo4j config
        if let Some(neo4j) = root.get("neo4j") {
            if let Some(uri) = neo4j.get("uri").and_then(|v| v.as_str()) {
                std::env::set_var("DEKA_NEO4J_URI", uri);
            }
            if let Some(user) = neo4j.get("user").and_then(|v| v.as_str()) {
                std::env::set_var("DEKA_NEO4J_USER", user);
            }
            if let Some(password) = neo4j.get("password").and_then(|v| v.as_str()) {
                std::env::set_var("DEKA_NEO4J_PASSWORD", password);
            }
            if let Some(db) = neo4j.get("db").and_then(|v| v.as_str()) {
                std::env::set_var("DEKA_NEO4J_DB", db);
            }
        }

        // Redis config
        if let Some(redis) = root.get("redis") {
            if let Some(url) = redis
                .get("url")
                .or_else(|| redis.get("uri"))
                .and_then(|v| v.as_str())
            {
                std::env::set_var("DEKA_REDIS_URL", url);
            }
        }
    }
}
