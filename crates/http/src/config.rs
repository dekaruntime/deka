//! Caller-supplied configuration for the HTTP layer (deka#801).
//!
//! Every value here used to be read from a `DEKA_*` process environment
//! variable at use time. Under deka#801 the environment is not a config
//! channel: the caller installs one `HttpConfig` when building the server and
//! the request path never touches the process environment.

use std::path::PathBuf;

use crate::rate_limit::RateLimitConfig;

/// Connection settings for the built-in `/api/*` platform handler
/// (`crate::api`). Replaces the `DEKA_NEO4J_URI` / `DEKA_NEO4J_USER` /
/// `DEKA_NEO4J_PASSWORD` / `DEKA_NEO4J_DB` reads (deka#801).
#[derive(Clone, Debug)]
pub struct Neo4jConfig {
    pub uri: String,
    pub user: String,
    pub password: String,
    pub db: String,
}

impl Default for Neo4jConfig {
    fn default() -> Self {
        Self {
            uri: "bolt://localhost:7687".to_string(),
            user: "neo4j".to_string(),
            password: String::new(),
            db: "neo4j".to_string(),
        }
    }
}

/// Server-wide HTTP configuration, installed once by the caller.
#[derive(Clone, Debug)]
pub struct HttpConfig {
    pub rate_limit: RateLimitConfig,
    /// Per-request `[http]` debug logging. Replaces `DEKA_HTTP_DEBUG` (deka#801).
    pub debug: bool,
    /// Activate the built-in `/api/*` platform handler. Replaces
    /// `DEKA_PLATFORM_API` (deka#801).
    pub platform_api: bool,
    pub neo4j: Neo4jConfig,
    /// Redis URL for the pageview tracker. Replaces `DEKA_REDIS_URL` (deka#801).
    pub redis_url: String,
    /// Project root used to discover `deka.css.json` for utility CSS.
    /// `None` keeps the built-in defaults. Replaces `DEKA_PROJECT_ROOT`
    /// (deka#801).
    pub project_root: Option<PathBuf>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            rate_limit: RateLimitConfig::default(),
            debug: false,
            platform_api: false,
            neo4j: Neo4jConfig::default(),
            redis_url: "redis://localhost:6379".to_string(),
            project_root: None,
        }
    }
}
