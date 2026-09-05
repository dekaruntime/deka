//! Compile `client:*` island modules into content-hashed `/assets/islands-{directive}.js`.
//!
//! Never copies `ui/server`. Chunks register islands then call `hydrate()`.
//!
//! Every emitted file is content-addressed: `<stem>.<sha256-10>.js`, where
//! `<sha256-10>` is the first 10 hex chars of the sha256 of the file bytes.
//! `deka build` (dist) and `deka serve` (.cache) both call into these
//! writers, so identical source yields identical names on both paths.
//!
//! Alongside the chunks this module writes `importmap.json` (RFD 24 §10.7):
//! the logical specifiers (`ui/jsx`, `ui/client`, `islands/load`, ...) map
//! to the hashed `/assets/...` URLs. `write_defer_client_assets` merges into
//! the same file, so it must run after `write_island_client_assets`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use runtime_core::framework::ClientIsland;

/// Number of hex chars of the sha256 digest appended into asset file names.
pub const ASSET_HASH_LEN: usize = 10;

/// Import map file written into the assets dir next to the hashed chunks.
pub const CLIENT_IMPORTMAP_FILE: &str = "importmap.json";

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

/// Logical specifiers owned by the client-asset import map. Entries under
/// these keys are dropped before each rewrite so removed islands/defer
/// loaders do not leave stale URLs behind; foreign keys are preserved.
const IMPORTMAP_OWNED_KEYS: [&str; 8] = [
    "ui/jsx",
    "ui/reactive",
    "ui/client",
    "ui/server",
    "islands/load",
    "islands/idle",
    "islands/visible",
    "islands/defer",
];

/// Short content hash (first `ASSET_HASH_LEN` hex chars of sha256).
pub fn content_hash_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(ASSET_HASH_LEN);
    for &byte in digest.iter().take(ASSET_HASH_LEN / 2) {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Content-addressed file name: `<stem>.<hash>.<ext>`.
pub fn hashed_asset_name(stem: &str, ext: &str, bytes: &[u8]) -> String {
    format!("{stem}.{}.{}", content_hash_hex(bytes), ext)
}

/// True when `name` is `<stem>.<ASSET_HASH_LEN lowercase-hex>.<ext>`.
fn is_hashed_asset_name(name: &str, ext: &str) -> bool {
    let Some(without_ext) = name.strip_suffix(ext) else {
        return false;
    };
    let Some((stem, hash)) = without_ext.strip_suffix('.').unwrap_or("").rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && hash.len() == ASSET_HASH_LEN
        && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Remove content-hashed `.js` files in `dir` that start with one of
/// `prefixes` and are not in `keep`. Unhashed files (and anything not
/// matching the hashed naming scheme) are left alone: they may belong to
/// fixtures or other tooling.
fn clean_stale_hashed_assets(
    dir: &Path,
    prefixes: &[&str],
    keep: &BTreeSet<String>,
) -> Result<(), String> {
    let Ok(reader) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in reader.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let looks_hashed = name.ends_with(".js") && is_hashed_asset_name(&name, "js");
        if looks_hashed
            && prefixes.iter().any(|prefix| name.starts_with(prefix))
            && !keep.contains(&name)
        {
            fs::remove_file(entry.path()).map_err(|err| {
                format!(
                    "failed to remove stale asset {}: {err}",
                    entry.path().display()
                )
            })?;
        }
    }
    Ok(())
}

/// Merge `entries` into `<assets_dir>/importmap.json`, preserving foreign keys.
fn write_importmap_entries(
    assets_dir: &Path,
    entries: &BTreeMap<String, String>,
) -> Result<(), String> {
    let path = assets_dir.join(CLIENT_IMPORTMAP_FILE);
    let mut imports: BTreeMap<String, String> = BTreeMap::new();
    if path.is_file() {
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(obj) = value.get("imports").and_then(|v| v.as_object()) {
                for (key, value) in obj {
                    if let Some(url) = value.as_str() {
                        imports.insert(key.clone(), url.to_string());
                    }
                }
            }
        }
    }
    for key in IMPORTMAP_OWNED_KEYS {
        imports.remove(key);
    }
    for (key, url) in entries {
        imports.insert(key.clone(), url.clone());
    }
    let json = serde_json::to_string_pretty(&serde_json::json!({ "imports": imports }))
        .unwrap_or_else(|_| "{\n  \"imports\": {}\n}".to_string());
    fs::write(&path, json.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

/// Map `/assets/<logical>.js|css` -> `/assets/<logical>.<hash>.js|css` for
/// every content-hashed file under `root` (recursively), where `dir` is the
/// directory currently being scanned. This is the single source of truth for
/// the logical->hashed URL rewrite shared by `deka build` (dist HTML) and
/// `deka serve` (serve-entry.dsx).
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

/// Rewrite the unhashed `/assets/...` logical URLs baked into the generated
/// app-router entry (`.cache/dekascript/serve-entry.dsx`) to the
/// content-hashed names emitted under `.cache/dekascript/assets`.
///
/// `deka serve` generates the entry before the client assets exist
/// (`engine::config::resolve_handler_path` runs ahead of the serve-startup
/// asset write), so the entry is born with unhashed names and this pass runs
/// immediately after the asset writers — the serve-side mirror of the
/// dist-HTML rewrite in `deka build` (crates/cli/src/cli/build.rs). Both
/// modes derive renames from the emitted files via
/// `collect_hashed_asset_renames`, so identical source resolves to identical
/// URLs in dev and prod. Idempotent: hashed URLs never match the logical
/// patterns.
///
/// The entry also carries the generation-time placeholder import-map tag
/// (`framework::CLIENT_IMPORTMAP_PLACEHOLDER_TAG`); this pass swaps it for
/// the inline map built from the freshly written `importmap.json`, so the
/// served document always carries the current hashes.
pub fn rewrite_serve_entry_asset_urls(project_root: &Path) -> Result<(), String> {
    let cache_dir = project_root.join(".cache").join("dekascript");
    let entry = cache_dir.join("serve-entry.dsx");
    if !entry.is_file() {
        return Ok(());
    }
    let assets_dir = cache_dir.join("assets");
    let mut renames: Vec<(String, String)> = Vec::new();
    collect_hashed_asset_renames(&assets_dir, &assets_dir, &mut renames)?;
    if renames.is_empty() && !assets_dir.join(CLIENT_IMPORTMAP_FILE).is_file() {
        return Ok(());
    }
    let mut source = fs::read_to_string(&entry)
        .map_err(|err| format!("failed to read {}: {err}", entry.display()))?;
    let mut changed = false;
    // External import maps are disallowed, so the placeholder (a `src`
    // reference) is replaced with the JSON body inline. The entry embeds the
    // document as JSON string literals (framework::json_str), so the tag
    // appears quote-escaped; swap that form, plus the plain form for
    // robustness.
    if let Some(tag) = inline_importmap_tag(&assets_dir)? {
        let placeholder = runtime_core::framework::CLIENT_IMPORTMAP_PLACEHOLDER_TAG;
        // JSON-escape the tag bodies (without the surrounding literal quotes)
        // the way framework::json_str embeds the document.
        let escaped_placeholder = placeholder.replace('"', "\\\"");
        let escaped_tag = tag.replace('"', "\\\"");
        for (from, to) in [
            (placeholder, tag.as_str()),
            (escaped_placeholder.as_str(), escaped_tag.as_str()),
        ] {
            if source.contains(from) {
                source = source.replace(from, to);
                changed = true;
            }
        }
    }
    for (logical, hashed) in &renames {
        if source.contains(logical.as_str()) {
            source = source.replace(logical.as_str(), hashed.as_str());
            changed = true;
        }
    }
    if changed {
        fs::write(&entry, source.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    }
    Ok(())
}

/// Hashed file names of the shared `ui/*` runtime chunks.
struct UiChunkNames {
    jsx: String,
    reactive: String,
    client: String,
    server_stub: String,
}

impl UiChunkNames {
    fn importmap_entries(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("ui/jsx".to_string(), format!("/assets/ui/{}", self.jsx)),
            (
                "ui/reactive".to_string(),
                format!("/assets/ui/{}", self.reactive),
            ),
            (
                "ui/client".to_string(),
                format!("/assets/ui/{}", self.client),
            ),
            (
                "ui/server".to_string(),
                format!("/assets/ui/{}", self.server_stub),
            ),
        ])
    }
}

/// Write the shared `ui/*` chunks under hashed names; returns the names.
fn write_ui_chunks(ui_dir: &Path) -> Result<UiChunkNames, String> {
    fs::create_dir_all(ui_dir)
        .map_err(|err| format!("failed to create {}: {err}", ui_dir.display()))?;
    // The vendored client chunk imports its siblings by unhashed relative
    // paths ("./jsx.js"); only hashed names exist on disk, so rewrite the
    // imports to the hashed sibling names before hashing the chunk itself —
    // otherwise the emitted graph 404s the moment it loads in a browser.
    let jsx = hashed_asset_name("jsx", "js", deka_ui::JSX.as_bytes());
    let reactive = hashed_asset_name("reactive", "js", deka_ui::REACTIVE.as_bytes());
    let client_src = deka_ui::CLIENT
        .replace("./jsx.js", &format!("./{jsx}"))
        .replace("./reactive.js", &format!("./{reactive}"));
    let names = UiChunkNames {
        jsx,
        reactive,
        client: hashed_asset_name("client", "js", client_src.as_bytes()),
        server_stub: hashed_asset_name(
            "server-stub",
            "js",
            b"export function renderToString() { throw new Error(\"ui/server is not available in the browser\"); }\n",
        ),
    };
    let keep: BTreeSet<String> = [
        names.jsx.clone(),
        names.reactive.clone(),
        names.client.clone(),
        names.server_stub.clone(),
    ]
    .into_iter()
    .collect();
    for (name, source) in [
        (&names.jsx, deka_ui::JSX.to_string()),
        (&names.reactive, deka_ui::REACTIVE.to_string()),
        (&names.client, client_src),
        (
            &names.server_stub,
            "export function renderToString() { throw new Error(\"ui/server is not available in the browser\"); }\n".to_string(),
        ),
    ] {
        fs::write(ui_dir.join(name), source.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", ui_dir.join(name).display()))?;
    }
    clean_stale_hashed_assets(ui_dir, &["jsx.", "reactive.", "client.", "server-stub."], &keep)?;
    Ok(names)
}

pub fn find_app_router_root(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        if runtime_core::framework::is_app_router_project(&cur) {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

pub fn write_island_client_assets_for_project(project_root: &Path) -> Result<(), String> {
    if !runtime_core::framework::is_app_router_project(project_root) {
        return Ok(());
    }
    let app_dir = project_root.join("app");
    let islands = runtime_core::framework::scan_client_islands(&app_dir);
    let deferred = runtime_core::framework::scan_server_defer(&app_dir);
    if islands.is_empty() && deferred.is_empty() {
        return Ok(());
    }
    write_island_client_assets(
        &project_root.join(".cache").join("dekascript").join("assets"),
        &islands,
    )?;
    if !deferred.is_empty() {
        write_defer_client_assets(
            &project_root.join(".cache").join("dekascript").join("assets"),
        )?;
    }
    Ok(())
}

pub fn write_island_client_assets(
    assets_dir: &Path,
    islands: &[ClientIsland],
) -> Result<(), String> {
    let ui_names = write_ui_chunks(&assets_dir.join("ui"))?;
    let mut importmap_entries = ui_names.importmap_entries();
    let mut keep_root: BTreeSet<String> = BTreeSet::new();

    for directive in ["load", "idle", "visible"] {
        let group: Vec<&ClientIsland> = islands
            .iter()
            .filter(|island| island.directive == directive)
            .collect();
        if group.is_empty() {
            continue;
        }
        let mut imports = String::new();
        let mut registers = String::new();
        let mut seen_files = BTreeSet::new();
        let mut idx = 0usize;
        for island in &group {
            if !seen_files.insert(island.file.clone()) {
                continue;
            }
            let source = fs::read_to_string(&island.file).map_err(|err| {
                format!("failed to read {}: {err}", island.file)
            })?;
            let mut js = compile_js(&source, &island.file)?;
            js = rewrite_ui_imports(&js, &ui_names);
            let mod_stem = format!("island-{directive}-{idx}");
            idx += 1;
            let names: Vec<&str> = group
                .iter()
                .filter(|item| item.file == island.file)
                .map(|item| item.component.as_str())
                .collect();
            let mut unique = Vec::new();
            for name in names {
                if !unique.contains(&name) {
                    unique.push(name);
                }
            }
            let spec_list = unique.join(", ");
            let mut missing = Vec::new();
            let mut to_export = Vec::new();
            for name in &unique {
                if js_exports_ident(&js, name) {
                    continue;
                }
                if js_has_ident_binding(&js, name) {
                    to_export.push(*name);
                } else {
                    missing.push(*name);
                }
            }
            if !missing.is_empty() {
                return Err(format!(
                    "client island {} is not exported from {}",
                    missing.join(", "),
                    island.file
                ));
            }
            if !to_export.is_empty() {
                js.push_str(&format!("\nexport {{ {} }};\n", to_export.join(", ")));
            }
            let mod_name = hashed_asset_name(&mod_stem, "js", js.as_bytes());
            fs::write(assets_dir.join(&mod_name), js.as_bytes()).map_err(|err| {
                format!("failed to write {}: {err}", assets_dir.join(&mod_name).display())
            })?;
            keep_root.insert(mod_name.clone());
            imports.push_str(&format!(
                "import {{ {spec_list} }} from \"./{mod_name}\";\n"
            ));
            for name in unique {
                registers.push_str(&format!(
                    "registerIsland({name_json}, {name});\n",
                    name_json = serde_json::to_string(name).unwrap_or_else(|_| format!("\"{name}\"")),
                    name = name
                ));
            }
        }
        let entry = format!(
            "{imports}import {{ hydrate, registerIsland }} from \"./ui/{}\";\n{registers}hydrate();\n",
            ui_names.client
        );
        let entry_name = hashed_asset_name(&format!("islands-{directive}"), "js", entry.as_bytes());
        let dest = assets_dir.join(&entry_name);
        fs::write(&dest, entry.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", dest.display()))?;
        keep_root.insert(entry_name.clone());
        importmap_entries.insert(
            format!("islands/{directive}"),
            format!("/assets/{entry_name}"),
        );
    }
    clean_stale_hashed_assets(
        assets_dir,
        &[
            "islands-load.",
            "islands-idle.",
            "islands-visible.",
            "island-load-",
            "island-idle-",
            "island-visible-",
        ],
        &keep_root,
    )?;
    write_importmap_entries(assets_dir, &importmap_entries)?;
    Ok(())
}

fn compile_js(source: &str, path: &str) -> Result<String, String> {
    match deka_compile::compile_to_js(source, path) {
        Ok(result) => Ok(result.js),
        Err(diagnostics) => Err(diagnostics
            .iter()
            .map(deka_compile::format_diagnostic)
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

pub fn write_defer_client_assets(assets_dir: &Path) -> Result<(), String> {
    let ui_names = write_ui_chunks(&assets_dir.join("ui"))?;
    let entry = format!(
        "import {{ hydrate }} from \"./ui/{}\";\nhydrate();\n",
        ui_names.client
    );
    let entry_name = hashed_asset_name("islands-defer", "js", entry.as_bytes());
    let dest = assets_dir.join(&entry_name);
    fs::write(&dest, entry.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", dest.display()))?;
    let keep: BTreeSet<String> = [entry_name.clone()].into_iter().collect();
    clean_stale_hashed_assets(assets_dir, &["islands-defer."], &keep)?;
    let mut importmap_entries = ui_names.importmap_entries();
    importmap_entries.insert("islands/defer".to_string(), format!("/assets/{entry_name}"));
    // Merges into the importmap written by `write_island_client_assets`; the
    // ui/* entries are identical, so call order (islands first) is preserved.
    write_importmap_entries(assets_dir, &importmap_entries)?;
    Ok(())
}

fn ident_boundary_after(src: &str, _name: &str) -> bool {
    match src.chars().next() {
        None => true,
        Some(c) => !c.is_ascii_alphanumeric() && c != '_',
    }
}

fn contains_prefixed_ident(src: &str, prefix: &str, name: &str) -> bool {
    let needle = format!("{prefix}{name}");
    let mut rest = src;
    while let Some(at) = rest.find(&needle) {
        let before_ok = at == 0
            || rest[..at]
                .chars()
                .next_back()
                .map(|c| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(true);
        let after = &rest[at + needle.len()..];
        if before_ok && ident_boundary_after(after, name) {
            return true;
        }
        rest = &rest[at + 1..];
    }
    false
}

fn js_exports_ident(js: &str, name: &str) -> bool {
    contains_prefixed_ident(js, "export function ", name)
        || contains_prefixed_ident(js, "export async function ", name)
        || contains_prefixed_ident(js, "export const ", name)
        || contains_prefixed_ident(js, "export let ", name)
        || contains_prefixed_ident(js, "export var ", name)
        || js_export_list_contains(js, name)
}

fn js_has_ident_binding(js: &str, name: &str) -> bool {
    js_exports_ident(js, name)
        || contains_prefixed_ident(js, "function ", name)
        || contains_prefixed_ident(js, "async function ", name)
        || contains_prefixed_ident(js, "const ", name)
        || contains_prefixed_ident(js, "let ", name)
        || contains_prefixed_ident(js, "var ", name)
}

fn js_export_list_contains(js: &str, name: &str) -> bool {
    let mut rest = js;
    while let Some(at) = rest.find("export {") {
        let after = &rest[at + "export {".len()..];
        let Some(end) = after.find('}') else {
            break;
        };
        for part in after[..end].split(',') {
            let ident = part.trim().split_whitespace().next().unwrap_or("");
            if ident == name {
                return true;
            }
        }
        rest = &after[end.saturating_add(1)..];
    }
    false
}

fn rewrite_ui_imports(js: &str, ui: &UiChunkNames) -> String {
    js.replace("from \"ui/jsx\"", &format!("from \"./ui/{}\"", ui.jsx))
        .replace("from \"ui/reactive\"", &format!("from \"./ui/{}\"", ui.reactive))
        .replace("from \"ui/client\"", &format!("from \"./ui/{}\"", ui.client))
        .replace("from \"ui/form\"", "from \"./ui/form.js\"")
        .replace("from \"ui/suspense\"", "from \"./ui/suspense.js\"")
        .replace("from \"ui/router\"", "from \"./ui/router.js\"")
        .replace(
            "from \"ui/server\"",
            &format!("from \"./ui/{}\"", ui.server_stub),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_short_hex() {
        let hash = content_hash_hex(b"deka");
        assert_eq!(hash.len(), ASSET_HASH_LEN);
        assert!(hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_eq!(hash, content_hash_hex(b"deka"));
        assert_ne!(hash, content_hash_hex(b"deka!"));
    }

    #[test]
    fn hashed_name_round_trips() {
        let name = hashed_asset_name("islands-load", "js", b"chunk");
        assert!(is_hashed_asset_name(&name, "js"), "{name}");
        assert!(!is_hashed_asset_name("islands-load.js", "js"));
        assert!(!is_hashed_asset_name("islands-load.JS", "js"));
        assert!(!is_hashed_asset_name(&name, "css"));
    }

    #[test]
    fn rewrite_ui_imports_targets_hashed_chunks() {
        let ui = UiChunkNames {
            jsx: "jsx.a1b2c3d4e5.js".to_string(),
            reactive: "reactive.a1b2c3d4e5.js".to_string(),
            client: "client.a1b2c3d4e5.js".to_string(),
            server_stub: "server-stub.a1b2c3d4e5.js".to_string(),
        };
        let js = rewrite_ui_imports(
            "import { x } from \"ui/jsx\";\nimport { y } from \"ui/server\";\n",
            &ui,
        );
        assert!(js.contains("from \"./ui/jsx.a1b2c3d4e5.js\""), "{js}");
        assert!(js.contains("from \"./ui/server-stub.a1b2c3d4e5.js\""), "{js}");
    }

    #[test]
    fn island_assets_are_content_addressed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let island_src = tmp.path().join("counter.dsx");
        fs::write(&island_src, "export fn Counter() {\n    return <button>0</button>;\n}\n")
            .expect("write island");
        let island = ClientIsland {
            component: "Counter".to_string(),
            directive: "load".to_string(),
            file: island_src.to_string_lossy().into_owned(),
            props: vec![],
        };
        let first = tmp.path().join("first");
        write_island_client_assets(&first, &[island.clone()]).expect("first write");
        let second = tmp.path().join("second");
        write_island_client_assets(&second, &[island.clone()]).expect("second write");
        let names = |dir: &Path| -> BTreeSet<String> {
            fs::read_dir(dir)
                .expect("read dir")
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(names(&first), names(&second), "same input -> same names");

        // Changing the island source changes its hash; ui/* chunks stay put.
        fs::write(&island_src, "export fn Counter() {\n    return <button>1</button>;\n}\n")
            .expect("rewrite island");
        let third = tmp.path().join("third");
        write_island_client_assets(&third, &[island]).expect("third write");
        let before = names(&first);
        let after = names(&third);
        assert_ne!(before, after, "changed island -> changed names");
        let ui_before: BTreeSet<String> = before
            .iter()
            .filter(|n| n.starts_with("jsx.") || n.starts_with("client."))
            .cloned()
            .collect();
        let ui_after: BTreeSet<String> = after
            .iter()
            .filter(|n| n.starts_with("jsx.") || n.starts_with("client."))
            .cloned()
            .collect();
        assert_eq!(ui_before, ui_after, "shared chunks must not rotate");

        // The importmap tracks the hashed URLs for both runs.
        let map: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(first.join(CLIENT_IMPORTMAP_FILE)).expect("read importmap"),
        )
        .expect("parse importmap");
        let imports = map["imports"].as_object().expect("imports object");
        let entry_url = imports["islands/load"].as_str().expect("islands/load url");
        let entry_name = entry_url.trim_start_matches("/assets/");
        assert!(first.join(entry_name).is_file(), "importmap url must exist: {entry_url}");
        assert!(entry_name.contains('.'), "importmap url must be hashed: {entry_url}");
    }

    #[test]
    fn importmap_tag_is_inlined_without_src() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // No map on disk -> no tag.
        assert!(inline_importmap_tag(tmp.path())
            .expect("read missing map")
            .is_none());

        write_importmap_entries(
            tmp.path(),
            &BTreeMap::from([("ui/client".to_string(), "/assets/ui/client.a1b2c3d4e5.js".to_string())]),
        )
        .expect("write importmap");
        let tag = inline_importmap_tag(tmp.path())
            .expect("build tag")
            .expect("tag for existing map");
        assert!(tag.starts_with(r#"<script type="importmap">"#), "{tag}");
        assert!(tag.ends_with("</script>"), "{tag}");
        assert!(
            !tag.contains("src="),
            "browsers reject the src form of <script type=\"importmap\">: {tag}"
        );
        let body = tag
            .strip_prefix(r#"<script type="importmap">"#)
            .and_then(|rest| rest.strip_suffix("</script>"))
            .expect("tag body");
        let map: serde_json::Value = serde_json::from_str(body).expect("inline body parses as JSON");
        let imports = map["imports"].as_object().expect("imports object");
        assert_eq!(
            imports["ui/client"].as_str().expect("ui/client url"),
            "/assets/ui/client.a1b2c3d4e5.js"
        );
    }

    #[test]
    fn rewrite_swaps_placeholder_for_inline_map() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache = tmp.path().join(".cache").join("dekascript");
        let assets = cache.join("assets");
        fs::create_dir_all(&assets).expect("mkdir assets");
        let island_src = assets.join("..").join("counter.dsx");
        fs::write(&island_src, "export fn Counter() {\n    return <button>0</button>;\n}\n")
            .expect("write island");
        let island = ClientIsland {
            component: "Counter".to_string(),
            directive: "load".to_string(),
            file: island_src.to_string_lossy().into_owned(),
            props: vec![],
        };
        write_island_client_assets(&assets, &[island]).expect("write assets");

        // The entry as generated: the document is embedded as JSON string
        // literals (framework::json_str), so the placeholder appears
        // quote-escaped, and the island script still has its logical URL.
        let entry = cache.join("serve-entry.dsx");
        let placeholder = runtime_core::framework::CLIENT_IMPORTMAP_PLACEHOLDER_TAG;
        let doc = format!(
            "<head>{placeholder}</head><script type=\"module\" src=\"/assets/islands-load.js\"></script>"
        );
        let doc_literal = serde_json::to_string(&doc).expect("encode doc literal");
        fs::write(&entry, format!("const doc_head = {doc_literal};\n")).expect("write entry");

        rewrite_serve_entry_asset_urls(tmp.path()).expect("rewrite");
        let served = fs::read_to_string(&entry).expect("read rewritten entry");
        assert!(
            !served.contains(placeholder) && !served.contains(r#"<script type=\"importmap\" src=\"#),
            "placeholder src-reference must be swapped out: {served}"
        );
        // Unescape the JSON literal and assert on the document itself.
        let served_doc = served.replace("\\\"", "\"");
        assert!(
            !served_doc.contains(r#"<script type="importmap" src="#),
            "no external import map may survive: {served_doc}"
        );
        let tag_start = served_doc
            .find(r#"<script type="importmap">"#)
            .expect("inline tag present");
        let body = &served_doc[tag_start + r#"<script type="importmap">"#.len()..];
        let body = &body[..body.find("</script>").expect("tag closes")];
        let map: serde_json::Value = serde_json::from_str(body).expect("inline body parses");
        let imports = map["imports"].as_object().expect("imports object");
        let ui_client = imports["ui/client"].as_str().expect("ui/client mapped");
        assert!(ui_client.starts_with("/assets/ui/client."), "hashed: {ui_client}");
        assert!(
            !served_doc.contains("/assets/islands-load.js"),
            "logical URL rewritten: {served_doc}"
        );
    }
}
