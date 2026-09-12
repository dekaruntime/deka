//! Content-hashed asset URL collection and the inline import-map tag.
//!
//! `collect_hashed_asset_renames` maps `/assets/<logical>.js|css` ->
//! `/assets/<logical>.<hash>.js|css` for every content-hashed file under a
//! root. It is the single source of truth for the logical->hashed URL rewrite
//! shared by `deka build` (dist HTML and compiled server entries) and the
//! web-bootstrap path.

use std::fs;
use std::path::{Path, PathBuf};

/// Number of hex chars of the sha256 digest appended into asset file names.
const ASSET_HASH_LEN: usize = 10;

/// Import map file written into the assets dir next to the hashed chunks.
const CLIENT_IMPORTMAP_FILE: &str = "importmap.json";

/// Read `<assets_dir>/importmap.json` and build the inline import-map tag for
/// it, or `None` when no map has been written. Browsers reject the `src` form
/// of this element (the attribute is disallowed on `<script type="importmap">`),
/// so the JSON body must be inlined into the document; the on-disk file stays
/// as the machine-readable copy for tooling and tests.
pub fn inline_importmap_tag(assets_dir: &Path) -> Result<Option<String>, String> {
    let path = assets_dir.join(CLIENT_IMPORTMAP_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    if value.get("imports").and_then(|v| v.as_object()).is_none() {
        return Err(format!("{} has no \"imports\" object", path.display()));
    }
    // Compact, and `<` escaped so the embedded JSON can never terminate the
    // script element early (escapes are valid JSON).
    let json = serde_json::to_string(&value)
        .map_err(|err| format!("failed to serialize {}: {err}", path.display()))?
        .replace('<', "\\u003c");
    Ok(Some(format!("<script type=\"importmap\">{json}</script>")))
}

/// Map `/assets/<logical>.js|css` -> `/assets/<logical>.<hash>.js|css` for
/// every content-hashed file under `root` (recursively), where `dir` is the
/// directory currently being scanned.
pub fn collect_hashed_asset_renames(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let Ok(reader) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in reader.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_hashed_asset_renames(root, &path, out)?;
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((stem_with_hash, ext)) = name.rsplit_once('.') else {
            continue;
        };
        if ext != "js" && ext != "css" {
            continue;
        }
        let Some((logical_stem, hash)) = stem_with_hash.rsplit_once('.') else {
            continue;
        };
        let is_hash = hash.len() == ASSET_HASH_LEN
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !is_hash {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let mut logical_rel = format!("{logical_stem}.{ext}");
        if let Some(parent) = Path::new(&rel).parent().and_then(|p| p.to_str()) {
            if !parent.is_empty() {
                logical_rel = format!("{parent}/{logical_rel}");
            }
        }
        out.push((format!("/assets/{logical_rel}"), format!("/assets/{rel}")));
    }
    Ok(())
}

/// Walk up from `start` to the nearest directory that is a source app-router
/// project. Used by `deka serve` to locate the `public/` dir for
/// source-posture projects.
pub fn find_app_router_root(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        if runtime_core::dist::is_source_app_router_project(&cur) {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}
