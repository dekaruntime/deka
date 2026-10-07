//! RFD 24 app-router dist pipeline, split by job:
//!
//! - `manifest` — filesystem → route entries (no compiler deps)
//! - `routes` — route data → slugs, segments, layout chains, matching (no
//!   compiler deps)
//! - `source` — `.ds`/`.dsx` text → exports, directives (shared
//!   scanning helpers)
//! - `defer` — `server:defer` scanning + §9.3 lints
//! - `document` — the `index.html` hole contract
//! - `codegen` — manifest → generated entry source (string templating until
//!   deka#391 phase (b) builds a `Program` and calls `emit_js`)
//!
//! Trailing-slash redirect policy lives in `engine::dispatch`, its only
//! caller; everything previously public here keeps its
//! `runtime_core::dist::*` path via these re-exports.

use std::path::{Path, PathBuf};

use deka_modules::modules::MODULES_DIR;

mod artifact_manifest;
mod build_invalidation;
mod build_manifest;
mod codegen;
mod defer;
mod document;
mod islands;
mod manifest;
mod routes;
mod source;

pub use build_invalidation::{SlotInvalidation, affected_slots, project_relative_path};
pub use build_manifest::{
    BuildManifest, BuildPlan, BuildPlanSlot, CompilerProvenance, FsObservation, FsObservationKind,
    ManifestArtifact, ManifestRoute, ManifestSlot, PlannedSource, RouteMode,
    SUPPORTED_PLAN_VERSIONS, sha256_hex, validate_plans,
};
pub use codegen::{
    generate_api_entry_source, generate_app_router_entry_source, generate_defer_entry_source,
    resolve_app_router_index_html, write_api_router_entry, write_app_router_entry,
    write_defer_router_entry, write_worker_router_entry,
};

/// Serve/dev compiler artifact directory.
///
/// Callers that need the development cache must select it explicitly with
/// [`compiler_cache_dir_with`]. The default is the production cache.
pub fn compiler_cache_dir(project_root: &Path) -> PathBuf {
    compiler_cache_dir_with(project_root, false)
}

/// deka#1065 found the compiler cache split across two physical roots at
/// once: `write_app_router_entry` always wrote `serve-entry.dsx` etc. to a
/// top-level `<root>/.cache/dekascript` regardless of dev/prod, while dev's
/// build-manifest/build-values tracking (crates/dev, deka_build::slots)
/// wrote to `<root>/ds_modules/.cache/dev` -- two roots existing
/// simultaneously during one `deka dev` session, which is exactly the bug
/// report.
///
/// Sami's call: the cache lives under `ds_modules/.cache` (never a
/// top-level `<root>/.cache` -- that pollutes the project root), and both
/// dev and prod resolve through this one function so there is exactly one
/// canonical root, never two. `ds_modules`'s existence still has to mean
/// "has installed packages" elsewhere (module-root / security-policy
/// resolution, and what `deka build` ships into `dist/server/ds_modules`)
/// -- that is handled by making those call sites test for actual installed
/// content rather than bare directory existence
/// (deka_host::validation::modules::existing_populated_modules_dirs) and by
/// having `deka build`'s `ds_modules` copy skip `.cache`
/// (deka_build::command::copy_dir_recursive), not by keeping the cache out
/// of `ds_modules`.
pub fn compiler_cache_dir_with(project_root: &Path, dev_mode: bool) -> PathBuf {
    let variant = if dev_mode { "dev" } else { "prod" };
    project_root.join(MODULES_DIR).join(".cache").join(variant)
}
pub use artifact_manifest::{
    ARTIFACT_FORMAT, ArtifactClient, ArtifactCompat, ArtifactManifestV2, ArtifactPayload,
    ArtifactProducer, ArtifactRoute, ArtifactServer, ArtifactSlot, ArtifactWorker, MODULE_FORMAT,
    PayloadRole, RUNTIME_ABI, ServerEntry, ServerEntryKind, artifact_digest, client_output_path,
    compute_payload_root, resolve_authored_artifact_root, server_entries, source_to_server_module,
};
pub use defer::{
    DeferLint, DeferLintLevel, DeferredIsland, defer_script_tag, scan_defer_lints,
    scan_server_defer,
};
pub use islands::{
    ClientIsland, ISLANDS_SCRIPT_SRC, generate_islands_entry_js, islands_script_tag,
    scan_client_islands,
};
pub use document::{
    CLIENT_IMPORTMAP_PLACEHOLDER_TAG, DEFAULT_INDEX_HARNESS, DEKA_APP_HOLE, DEKA_HEAD_HOLE,
    DEKA_SCRIPTS_HOLE, FRAGMENT_ACCEPT, FRAGMENT_ACCEPT_LEGACY,
};
pub use manifest::{
    FrameworkEntry, FrameworkEntryKind, FrameworkManifest, collect_public_rel_paths,
    exported_http_methods, is_built_app_router_project, is_source_app_router_project, scan_api_dir,
    scan_app_dir,
};
pub use routes::{
    RouteMatch, cloudflare_redirects, layout_chain, match_path, normalize_request_path,
    route_from_relative_path, route_pattern_matches,
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
    fn compiler_cache_dir_prod_nests_under_ds_modules() {
        let root = Path::new("/tmp/proj");
        assert_eq!(
            compiler_cache_dir_with(root, false),
            root.join("ds_modules").join(".cache").join("prod")
        );
    }

    #[test]
    fn compiler_cache_dir_dev_nests_under_ds_modules() {
        let root = Path::new("/tmp/proj");
        assert_eq!(
            compiler_cache_dir_with(root, true),
            root.join("ds_modules").join(".cache").join("dev")
        );
    }

    /// deka#1065: exactly one cache root, never a second one at the
    /// project's top level -- Sami does not want the cache polluting the
    /// project root. Both dev and prod resolve under the same
    /// `ds_modules/.cache` parent, split only by subdirectory.
    #[test]
    fn compiler_cache_dir_never_lands_at_project_root() {
        let root = Path::new("/tmp/proj");
        for dev_mode in [true, false] {
            let dir = compiler_cache_dir_with(root, dev_mode);
            assert!(
                dir.starts_with(root.join("ds_modules").join(".cache")),
                "cache dir must nest under ds_modules/.cache, got {}",
                dir.display()
            );
            assert_ne!(
                dir.parent().map(|p| p.to_path_buf()),
                Some(root.to_path_buf()),
                "cache dir must never be a direct child of the project root, got {}",
                dir.display()
            );
        }
    }
}
