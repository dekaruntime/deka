//! Process-wide host configuration installed by the dispatch layer
//! (deka#801). The process environment is not a configuration transport:
//! the dispatch layer (`crates/runtime`) installs the values it resolves
//! through these installer functions, and host modules read them back from
//! the installed store — never from `std::env::var`. The first install wins
//! (later `set` calls are ignored), matching the old set-if-absent
//! environment semantics.

use std::sync::OnceLock;

/// Database connection endpoints resolved from deka.json by the dispatch
/// layer. Replaces the `DEKA_NEO4J_*` process-environment
/// reads (deka#801); each consumer falls back to its hardcoded default when
/// a field is `None` or nothing has been installed.
#[derive(Debug, Clone, Default)]
pub struct DatabaseEndpoints {
    pub neo4j_uri: Option<String>,
    pub neo4j_user: Option<String>,
    pub neo4j_password: Option<String>,
    pub neo4j_db: Option<String>,
}

static DATABASE_ENDPOINTS: OnceLock<DatabaseEndpoints> = OnceLock::new();

/// Install the database endpoints for this process. Called once by the
/// dispatch layer after it resolves deka.json; later installs are ignored.
pub fn install_database_endpoints(endpoints: DatabaseEndpoints) {
    let _ = DATABASE_ENDPOINTS.set(endpoints);
}

/// The installed database endpoints, if the dispatch layer installed any.
pub fn database_endpoints() -> Option<&'static DatabaseEndpoints> {
    DATABASE_ENDPOINTS.get()
}

/// Handler paths the dispatch layer resolved for this process. Replaces the
/// `HANDLER_PATH` / `DEKA_MODULE_ROOT` process-environment reads in the
/// PHPX-legacy bridge (deka#801).
#[derive(Debug, Clone, Default)]
pub struct HandlerPaths {
    /// Path of the handler entry the dispatch layer is serving or running.
    pub handler_path: Option<String>,
    /// Explicit ds_modules project-root override, when the dispatch layer
    /// supplies one.
    pub module_root: Option<String>,
}

static HANDLER_PATHS: OnceLock<HandlerPaths> = OnceLock::new();

/// Install the handler paths for this process. Called once by the dispatch
/// layer after it resolves the handler entry; later installs are ignored.
pub fn install_handler_paths(paths: HandlerPaths) {
    let _ = HANDLER_PATHS.set(paths);
}

/// The installed handler paths, if the dispatch layer installed any.
pub fn handler_paths() -> Option<&'static HandlerPaths> {
    HANDLER_PATHS.get()
}
