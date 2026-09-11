//! Build-time `dist/server/` emission (deka#762).
//!
//! Owns the promotion of staged server output into the source-free,
//! loader-ready `dist/server/` product: creating the directory, compiling the
//! page/api/defer router entries through dsc and re-rooting their graph,
//! filling modules no entry imports, and moving the remaining emitted
//! app/src/api trees into place. Also bakes the final content-hashed client
//! asset URLs (and the built import map) into the compiled server entries
//! once every client asset exists — the built entry serves production, so
//! there is no serve-time rewrite pass for it.

use std::fs;
use std::path::Path;

use crate::cli::build::copy_dir_recursive;
use crate::cli::build_dsc;

pub(crate) struct ServerEntriesPlan<'a> {
    pub project_root: &'a Path,
    pub staging_root: &'a Path,
    pub entries_dir: &'a Path,
    pub dist_server: &'a Path,
    pub has_manifest: bool,
    pub emitted_src: bool,
    pub emitted_api: bool,
    /// Compiler selected by the CLI for this build invocation. This is not
    /// carried into serve: built artifacts must run without dsc.
    pub dsc: &'a Path,
}

pub(crate) fn emit_server_entries(plan: &ServerEntriesPlan<'_>) -> Result<(), String> {
    fs::create_dir_all(plan.dist_server)
        .map_err(|err| format!("failed to create {}: {}", plan.dist_server.display(), err))?;

    #[cfg(feature = "native")]
    if plan.has_manifest {
        let _emitted_entries = crate::cli::build_server_graph::compile_and_reroot_entries(
            plan.project_root,
            plan.entries_dir,
            plan.dist_server,
            plan.dsc,
        )?;
        // Fill in modules no entry imports (dynamic-import targets, non-DS
        // assets) from the emitted trees; graph modules already in place win.
        build_dsc::copy_non_ds_tree(
            &plan.staging_root.join("app"),
            &plan.dist_server.join("app"),
            true,
        )?;
        if plan.emitted_api {
            build_dsc::copy_non_ds_tree(
                &plan.staging_root.join("api"),
                &plan.dist_server.join("api"),
                true,
            )?;
        }
    }
    #[cfg(feature = "native")]
    if !plan.has_manifest {
        replace_dir(
            &plan.staging_root.join("app"),
            &plan.dist_server.join("app"),
        )?;
    }
    if plan.emitted_src {
        replace_dir(
            &plan.staging_root.join("src"),
            &plan.dist_server.join("src"),
        )?;
    }
    #[cfg(feature = "native")]
    if plan.emitted_api && !plan.has_manifest {
        replace_dir(
            &plan.staging_root.join("api"),
            &plan.dist_server.join("api"),
        )?;
    }
    Ok(())
}

fn replace_dir(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        fs::remove_dir_all(dst)
            .map_err(|err| format!("failed to remove {}: {err}", dst.display()))?;
    }
    if !src.is_dir() {
        return Ok(());
    }
    copy_dir_recursive(src, dst)
}

/// Bake the final content-hashed asset names (and the built import map, in
/// place of the generation-time placeholder) into the compiled server
/// entries. The renames and the inline tag come from the same collector the
/// dist-HTML and serve-entry rewrites use, so dev and prod agree by
/// construction.
pub(crate) fn rewrite_server_entry_asset_urls(
    dist_server: &Path,
    dist_client: &Path,
) -> Result<(), String> {
    let assets_dir = dist_client.join("assets");
    let mut renames: Vec<(String, String)> = Vec::new();
    runtime::collect_hashed_asset_renames(&assets_dir, &assets_dir, &mut renames)?;
    let importmap_tag = if assets_dir.join("importmap.json").is_file() {
        runtime::inline_importmap_tag(&assets_dir)?
    } else {
        None
    };
    for name in ["serve-entry.js", "api-entry.js", "defer-entry.js"] {
        let path = dist_server.join(name);
        if !path.is_file() {
            continue;
        }
        let mut js = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let mut changed = false;
        for (logical, hashed) in &renames {
            if js.contains(logical.as_str()) {
                js = js.replace(logical.as_str(), hashed.as_str());
                changed = true;
            }
        }
        if let Some(tag) = &importmap_tag {
            let placeholder = runtime_core::framework::CLIENT_IMPORTMAP_PLACEHOLDER_TAG;
            if js.contains(placeholder) {
                js = js.replace(placeholder, tag);
                changed = true;
            }
        }
        if changed {
            fs::write(&path, js.as_bytes())
                .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
        }
    }
    Ok(())
}
