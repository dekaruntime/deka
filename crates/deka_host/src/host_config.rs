//! Process-wide host configuration installed by the dispatch layer
//! (deka#801). The process environment is not a configuration transport:
//! the dispatch layer (`crates/runtime`) installs the values it resolves
//! through these installer functions, and host modules read them back from
//! the installed store — never from `std::env::var`. The first install wins
//! (later `set` calls are ignored), matching the old set-if-absent
//! environment semantics.

use std::sync::OnceLock;

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
