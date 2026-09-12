//! RFD 21 (deka#754): the closed `deka.*` catalog is stdlib-only. This module
//! is the loader half of the gate: which modules may bind the catalog through
//! the per-module preamble, and the static boot check that refuses a compiled
//! graph whose non-official modules reference a catalog kind (they could never
//! resolve their helpers). The source half of the gate lives in
//! [`crate::deka_catalog_scan`]; official packages were validated there
//! (unknown helpers, arity, classification) before dsc ran.

use std::path::{Path, PathBuf};

use super::PhpxEsmLoader;
use super::{dependency_package_name, read_manifest_name};
use permissions::host_bridge;

impl PhpxEsmLoader {
    /// Whether a module may bind the closed `deka.*` catalog through the
    /// per-module preamble. Same classification the source scan used before
    /// compile: official `@deka/*` dependencies, an official project root,
    /// the runtime-distribution `ui/*` modules, or a linked package whose
    /// nearest manifest is official. Application code never qualifies.
    pub(super) fn catalog_eligible_for_path(&self, path: &Path) -> bool {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path: &Path = &canonical;
        // Materialized `ui/*` toolchain modules are part of the deka
        // distribution like the prelude.
        if path.starts_with(self.cache_dir.join("ui")) {
            return true;
        }
        if let Some(package_root) = self.dependency_package_root(path) {
            let name = dependency_package_name(&package_root, &self.project_root);
            return host_bridge::is_official_package_name(&name);
        }
        if path.starts_with(&self.project_root) {
            return self.root_official;
        }
        // Linked packages outside the project root: nearest manifest decides.
        path.ancestors()
            .find(|ancestor| ancestor.join("deka.json").is_file())
            .and_then(|root| read_manifest_name(&root))
            .map(|name| host_bridge::is_official_package_name(&name))
            .unwrap_or(false)
    }
}

/// Boot gate over dsc's compiled modules: Some(message) when a non-official
/// module references a catalog kind — that graph can never run, so refuse to
/// boot instead of failing at first call.
pub(crate) fn non_official_catalog_reference(
    loader: &PhpxEsmLoader,
    modules: &std::collections::HashMap<PathBuf, String>,
) -> Option<String> {
    let mut paths: Vec<&PathBuf> = modules.keys().collect();
    paths.sort();
    for path in paths {
        if loader.catalog_eligible_for_path(path) {
            continue;
        }
        let js = &modules[path];
        let referenced = runtime_core::deka_catalog::DEKA_CATALOG
            .iter()
            .find(|kind| js.contains(&format!("deka.{}.", kind.name)));
        if let Some(kind) = referenced {
            let name = loader.package_name_for_path(path);
            return Some(format!(
                "package '{}' references the closed deka.* catalog (deka.{}) but is not an official @deka/* stdlib package ({})",
                name,
                kind.name,
                path.display()
            ));
        }
    }
    None
}
