//! RFD 24 app-router framework support, split by job:
//!
//! - `manifest` — filesystem → route entries (no compiler deps)
//! - `routes` — route data → slugs, segments, layout chains, matching (no
//!   compiler deps)
//! - `source` — `.ds`/`.dsx` text → exports, imports, directives (shared
//!   scanning helpers)
//! - `islands` / `defer` — client-island and `server:defer` scanning +
//!   §9.3 lints
//! - `css` — manifest + sources → scoped style plan
//! - `document` — the `index.html` hole contract
//! - `codegen` — manifest → generated entry source (string templating until
//!   deka#391 phase (b) builds a `Program` and calls `emit_js`)
//!
//! Trailing-slash redirect policy lives in `engine::dispatch`, its only
//! caller; everything previously public here keeps its
//! `runtime_core::framework::*` path via these re-exports.

use std::path::{Path, PathBuf};

use crate::env::env_truthy_with;
use crate::modules::MODULES_DIR;

mod codegen;
mod css;
mod defer;
mod document;
mod islands;
mod manifest;
mod routes;
mod source;

pub use codegen::{
    write_api_router_entry, write_app_router_entry, write_defer_router_entry,
    write_static_render_entry, write_worker_router_entry,
};

/// Serve/dev compiler artifact directory.
///
/// When `DEKA_DEV` is set (`deka` / `deka serve --dev`), artifacts go under
/// `ds_modules/.cache/dev`. Otherwise the legacy `.cache/dekascript` path is
/// used (also a build staging area). Production `dist/` is unchanged.
pub fn compiler_cache_dir(project_root: &Path) -> PathBuf {
    compiler_cache_dir_with(
        project_root,
        env_truthy_with("DEKA_DEV", &|key| std::env::var(key).ok()),
    )
}

pub fn compiler_cache_dir_with(project_root: &Path, dev_mode: bool) -> PathBuf {
    if dev_mode {
        project_root
            .join(MODULES_DIR)
            .join(".cache")
            .join("dev")
    } else {
        project_root.join(".cache").join("dekascript")
    }
}
pub use css::{
    CssPlan, RouteStyle, ScopedStyleFile, collect_class_literals, collect_route_styles,
    css_links_for_route, css_plan_from_styles, css_scope_hash,
};
pub use defer::{
    DeferLint, DeferLintLevel, DeferredIsland, defer_script_tag, scan_defer_lints,
    scan_server_defer,
};
pub use document::{
    CLIENT_IMPORTMAP_PLACEHOLDER_TAG, DEKA_APP_HOLE, DEKA_HEAD_HOLE, DEKA_SCRIPTS_HOLE,
    FRAGMENT_ACCEPT, FRAGMENT_ACCEPT_LEGACY, fill_document,
};
pub use islands::{ClientIsland, island_script_tags, scan_client_islands};
pub use manifest::{
    FrameworkEntry, FrameworkEntryKind, FrameworkManifest, collect_public_rel_paths,
    exported_http_methods, is_app_router_project, scan_api_dir, scan_app_dir,
};
pub use routes::{
    RouteMatch, cloudflare_redirects, layout_chain, match_path, normalize_request_path,
    route_css_slug, route_from_relative_path, route_pattern_matches, static_page_routes,
};
pub fn project_needs_worker(project_root: &Path) -> bool {
    !scan_api_dir(&project_root.join("api")).is_empty()
        || !scan_server_defer(&project_root.join("app")).is_empty()
}

#[cfg(test)]
mod cache_dir_tests {
    use super::compiler_cache_dir_with;
    use std::path::Path;

    #[test]
    fn compiler_cache_dir_defaults_to_legacy_path() {
        let root = Path::new("/tmp/proj");
        assert_eq!(
            compiler_cache_dir_with(root, false),
            root.join(".cache").join("dekascript")
        );
    }

    #[test]
    fn compiler_cache_dir_uses_ds_modules_when_dev() {
        let root = Path::new("/tmp/proj");
        assert_eq!(
            compiler_cache_dir_with(root, true),
            root.join("ds_modules").join(".cache").join("dev")
        );
    }
}
