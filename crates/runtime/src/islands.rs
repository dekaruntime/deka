//! Compile `client:*` island modules into content-hashed `/assets/islands-{directive}.js`.
//!
//! Never copies `ui/server`. Chunks register islands then call `hydrate()`.
//!
//! Every emitted file is content-addressed: `<stem>.<sha256-10>.js`, where
//! `<sha256-10>` is the first 10 hex chars of the sha256 of the final file
//! bytes. `deka build` (dist) and `deka serve` (.cache) both call into these
//! writers; within one [`ClientAssetFlavor`] identical source yields identical
//! names. The flavors differ by design since deka#750: dist output is
//! tree-shaken and minified, dev output stays readable, and content
//! addressing means their hashes differ.
//!
//! Alongside the chunks this module writes `importmap.json` (RFD 24 §10.7):
//! the logical specifiers (`ui/jsx`, `ui/client`, `islands/load`, ...) map
//! to the hashed `/assets/...` URLs. `write_defer_client_assets` merges into
//! the same file, so it must run after `write_island_client_assets`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use runtime_core::framework::ClientIsland;

/// How browser-bound client assets are emitted (deka#750).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClientAssetFlavor {
    /// `deka serve` / dev cache: readable, unpruned sources.
    Dev,
    /// `deka build` dist: tree-shaken + minified (deka#750 payload budget).
    Dist,
}

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

/// Browser stub for `ui/server`. Islands must not pull the real renderer.
const UI_SERVER_STUB: &str = concat!(
    "export function renderToString() { ",
    "throw new Error(\"ui/server is not available in the browser\"); ",
    "}\n",
);

/// Disk stem for the ui/server stub. Not `server`, so a leaked real
/// `server.js` cannot collide with the stub's hashed name.
const UI_SERVER_STUB_STEM: &str = "server-stub";

/// Island-chunk specifiers owned by the client-asset import map, alongside
/// every `deka_ui::SPECIFIERS` entry. Dropped before each rewrite so removed
/// islands/defer loaders (and a ui/* file that stops being written) do not
/// leave stale URLs behind; foreign keys are preserved.
const ISLAND_IMPORTMAP_KEYS: [&str; 4] = [
    "islands/load",
    "islands/idle",
    "islands/visible",
    "islands/defer",
];

/// Keys this writer owns: every compiler-provided `ui/*` specifier plus the
/// island/defer chunk keys. Derived from `deka_ui::SPECIFIERS` so a newly
/// added ui module is owned (and stale-dropped) without a second list.
fn importmap_owned_keys() -> impl Iterator<Item = &'static str> {
    deka_ui::SPECIFIERS
        .iter()
        .copied()
        .chain(ISLAND_IMPORTMAP_KEYS.iter().copied())
}

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
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
    for key in importmap_owned_keys() {
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
/// app-router entry (`compiler_cache_dir(...)/serve-entry.dsx`) to the
/// content-hashed names emitted under that cache's `assets/` directory.
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
    let cache_dir = runtime_core::framework::compiler_cache_dir(project_root);
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

/// One compiled `ui/*` module as shipped to the browser.
struct UiModule {
    specifier: &'static str,
    stem: &'static str,
    source: String,
}

/// Hashed file names of the shared `ui/*` runtime chunks, keyed by specifier
/// (`"ui/jsx"` → `"jsx.<hash>.js"`). The map is the single source the import
/// map, the island-module rewrite, and the on-disk writer all read from —
/// a specifier is rewritten if and only if it was written (deka#622 finding E).
struct UiChunkNames {
    hashed: BTreeMap<String, String>,
}

impl UiChunkNames {
    fn file(&self, spec: &str) -> &str {
        self.hashed
            .get(spec)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("ui specifier {spec} was not written"))
    }

    fn importmap_entries(&self) -> BTreeMap<String, String> {
        self.hashed
            .iter()
            .map(|(spec, name)| (spec.clone(), format!("/assets/ui/{name}")))
            .collect()
    }
}

/// Every compiler-provided `ui/*` module, with `ui/server` replaced by the
/// browser stub. Derived from `deka_ui::SPECIFIERS` so a newly added ui file
/// is written (and rewritten) without a second hand-maintained list.
fn client_ui_modules() -> Vec<UiModule> {
    deka_ui::SPECIFIERS
        .iter()
        .filter_map(|spec| {
            let file = deka_ui::file_name_for(spec)?;
            let stem = if *spec == "ui/server" {
                UI_SERVER_STUB_STEM
            } else {
                file.strip_suffix(".js")?
            };
            let source = if *spec == "ui/server" {
                UI_SERVER_STUB.to_string()
            } else {
                deka_ui::source_for(spec)?.to_string()
            };
            Some(UiModule {
                specifier: spec,
                stem,
                source,
            })
        })
        .collect()
}

/// Map unhashed sibling filenames (`jsx.js`) to the hashed names on disk.
fn hashed_by_unhashed_file(
    modules: &[UiModule],
    hashed: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for module in modules {
        let Some(file) = deka_ui::file_name_for(module.specifier) else {
            continue;
        };
        if let Some(name) = hashed.get(module.specifier) {
            out.insert(file.to_string(), name.clone());
        }
    }
    out
}

/// Rewrite `./jsx.js` (etc.) inside a ui chunk to the hashed sibling name.
fn rewrite_relative_ui_imports(source: &str, hashed_files: &BTreeMap<String, String>) -> String {
    let mut out = source.to_string();
    for (file, hashed) in hashed_files {
        let from = format!("./{file}");
        if out.contains(&from) {
            out = out.replace(&from, &format!("./{hashed}"));
        }
    }
    out
}

/// Hash each module, rewrite relative sibling imports to those hashed names,
/// and re-hash any module whose source changed. Leaves (no relative imports)
/// keep their original hash, so dependents that import them stabilize after
/// the first pass; a later sibling edge takes another pass, bounded by the
/// specifier count.
fn hash_ui_modules(modules: &mut [UiModule]) -> BTreeMap<String, String> {
    let mut hashed: BTreeMap<String, String> = modules
        .iter()
        .map(|module| {
            (
                module.specifier.to_string(),
                hashed_asset_name(module.stem, "js", module.source.as_bytes()),
            )
        })
        .collect();
    for _ in 0..deka_ui::SPECIFIERS.len() {
        let by_file = hashed_by_unhashed_file(modules, &hashed);
        let mut changed = false;
        for module in modules.iter_mut() {
            let rewritten = rewrite_relative_ui_imports(&module.source, &by_file);
            if rewritten != module.source {
                module.source = rewritten;
                hashed.insert(
                    module.specifier.to_string(),
                    hashed_asset_name(module.stem, "js", module.source.as_bytes()),
                );
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    hashed
}

/// Per-module export pruning plan for dist: `named` maps a ui-specifier to
/// the exported names its importers use; `keep_all` lists specifiers some
/// importer observed without naming bindings (namespace/default/side-effect
/// import, `export *`, dynamic `import()`), which must not be pruned at all.
/// `None` means "no pruning": some importer used direct `eval`/`with`/dynamic
/// scope, so any binding may be observed and everything must survive.
type UiKeepSets = Option<UiPrunePlan>;

struct UiPrunePlan {
    named: BTreeMap<String, HashSet<String>>,
    keep_all: HashSet<String>,
}

/// Synthetic import line every client entry issues: `hydrate` (and
/// `registerIsland` for the island entries) come from `ui/client`.
const CLIENT_ENTRY_IMPORTS: &str =
    "import { hydrate, registerIsland } from \"ui/client\";";

/// Optimize browser-bound source per flavor (deka#750): dist chunks are
/// export-pruned + minified; dev chunks pass through byte-identical.
fn optimize_client_chunk(
    name: &str,
    source: &str,
    flavor: ClientAssetFlavor,
) -> Result<String, String> {
    match flavor {
        ClientAssetFlavor::Dev => Ok(source.to_string()),
        ClientAssetFlavor::Dist => bundler::optimize_emitted_module(source, Path::new(name)),
    }
}

/// Map an import specifier (`"ui/jsx"` or the sibling form `"./jsx.js"`) to
/// the `deka_ui` specifier it resolves to, or `None` when it is not a shipped
/// client chunk (e.g. an island module import).
fn resolve_ui_import_target(spec: &str) -> Option<String> {
    for candidate in [spec.to_string(), format!("{spec}.js"), format!("{spec}.mjs")] {
        if deka_ui::source_for(&candidate).is_some() {
            return Some(candidate);
        }
    }
    let file = spec.rsplit(['/', '\\']).next().unwrap_or(spec);
    for known in deka_ui::SPECIFIERS {
        if deka_ui::file_name_for(known) == Some(file) {
            return Some((*known).to_string());
        }
    }
    None
}

/// Compute per-module export keep-sets across the complete set of modules a
/// browser can load: the given island-chunk sources (pre ui-rewrite), the
/// synthetic client entry import, and every ui module's own relative imports.
/// Returns `Ok(None)` when any importer uses dynamic scope — nothing is
/// prunable then. Importers outside this set (a hand-written module reaching
/// into a `ui/*` chunk) are unsupported: pruning assumes the keep-set here
/// is complete.
fn compute_ui_keep_sets(importer_sources: &[String]) -> Result<UiKeepSets, String> {
    let mut plan = UiPrunePlan {
        named: BTreeMap::new(),
        keep_all: HashSet::new(),
    };
    let mut scan = |source: &str, file: &str| -> Result<bool, String> {
        let scan = bundler::scan_module_imports(source, Path::new(file))?;
        if scan.uses_dynamic_scope {
            return Ok(true);
        }
        for (spec, use_) in scan.imports {
            let Some(target) = resolve_ui_import_target(&spec) else {
                continue;
            };
            match use_ {
                bundler::ImportUse::KeepAll => {
                    // Any binding may be observed: never prune this module.
                    plan.named.remove(&target);
                    plan.keep_all.insert(target);
                }
                bundler::ImportUse::Named(names) => {
                    if !plan.keep_all.contains(&target) {
                        plan.named.entry(target).or_default().extend(names);
                    }
                }
            }
        }
        Ok(false)
    };
    for source in importer_sources {
        if scan(source, "island.js")? {
            return Ok(None);
        }
    }
    if scan(CLIENT_ENTRY_IMPORTS, "client-entry.js")? {
        return Ok(None);
    }
    for module in client_ui_modules() {
        if scan(&module.source, &format!("{}.js", module.stem))? {
            return Ok(None);
        }
    }
    Ok(Some(plan))
}

/// Write the shared `ui/*` chunks under hashed names; returns the names. In
/// the `Dist` flavor each module is export-pruned to its keep-set and
/// minified before hashing; `Dev` writes the readable sources unchanged. A
/// module that prunes to nothing is still written as `export {};`: under the
/// complete-importer-set contract no known importer can reference it, and
/// keeping a non-empty file + importmap entry preserves dev/prod parity.
fn write_ui_chunks(
    ui_dir: &Path,
    flavor: ClientAssetFlavor,
    keep: &UiKeepSets,
) -> Result<UiChunkNames, String> {
    fs::create_dir_all(ui_dir)
        .map_err(|err| format!("failed to create {}: {err}", ui_dir.display()))?;
    let mut modules = client_ui_modules();
    if flavor == ClientAssetFlavor::Dist {
        for module in &mut modules {
            // Rewrite dsc-emitted match machinery first so pruning sees the
            // simplified bindings (the Result prelude half becomes unreachable
            // and drops; deka#771).
            module.source = bundler::simplify_emitted_module(
                &module.source,
                &Path::new(&format!("{}.js", module.stem)),
            )?;
            let pruned = match keep {
                Some(plan) if !plan.keep_all.contains(module.specifier) => {
                    bundler::prune_unreferenced_exports(
                        &module.source,
                        &Path::new(&format!("{}.js", module.stem)),
                        &plan.named.get(module.specifier).cloned().unwrap_or_default(),
                    )?
                }
                _ => module.source.clone(),
            };
            module.source =
                optimize_client_chunk(&format!("{}.js", module.stem), &pruned, flavor)?;
            if module.source.trim().is_empty() {
                // Pruned to nothing: still a valid, non-empty ES module so the
                // written file + importmap entry survive dev/prod parity and
                // the "assets are non-empty on disk" check.
                module.source = "export {};\n".to_string();
            }
        }
    }
    let hashed = hash_ui_modules(&mut modules);
    let keep: BTreeSet<String> = hashed.values().cloned().collect();
    for module in &modules {
        let name = hashed
            .get(module.specifier)
            .expect("every client ui module is hashed");
        fs::write(ui_dir.join(name), module.source.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", ui_dir.join(name).display()))?;
    }
    let prefixes: Vec<String> = modules
        .iter()
        .map(|module| format!("{}.", module.stem))
        .collect();
    let prefix_refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
    clean_stale_hashed_assets(ui_dir, &prefix_refs, &keep)?;
    Ok(UiChunkNames { hashed })
}

pub fn find_app_router_root(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        if runtime_core::framework::is_source_app_router_project(&cur) {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

pub fn write_island_client_assets_for_project(
    project_root: &Path,
    flavor: ClientAssetFlavor,
) -> Result<(), String> {
    write_island_client_assets_for_project_with_optional_dsc(project_root, flavor, None)
}

/// Build-only form with a compiler selected at the CLI boundary.
pub fn write_island_client_assets_for_project_with_dsc(
    project_root: &Path,
    flavor: ClientAssetFlavor,
    dsc: &Path,
) -> Result<(), String> {
    write_island_client_assets_for_project_with_optional_dsc(project_root, flavor, Some(dsc))
}

fn write_island_client_assets_for_project_with_optional_dsc(
    project_root: &Path,
    flavor: ClientAssetFlavor,
    dsc: Option<&Path>,
) -> Result<(), String> {
    if !runtime_core::framework::is_source_app_router_project(project_root) {
        return Ok(());
    }
    let app_dir = project_root.join("app");
    let islands = runtime_core::framework::scan_client_islands(&app_dir);
    let deferred = runtime_core::framework::scan_server_defer(&app_dir);
    if islands.is_empty() && deferred.is_empty() {
        return Ok(());
    }
    let assets_dir = runtime_core::framework::compiler_cache_dir(project_root).join("assets");
    write_island_client_assets_with_optional_dsc(&assets_dir, &islands, flavor, dsc)?;
    if !deferred.is_empty() {
        write_defer_client_assets(&assets_dir, flavor)?;
    }
    Ok(())
}

/// One compiled island module group (one per unique source file per
/// directive), before its `ui/*` imports are rewritten to hashed names.
struct IslandChunk {
    directive: &'static str,
    mod_stem: String,
    file: String,
    /// Compiled source with the `export { ... }` append already applied.
    js: String,
    /// Unique component names exported for this file's group.
    names: Vec<String>,
}

/// Compile every island source file exactly once per directive, appending
/// `export { ... }` for bound-but-unexported components. Keep-sets for dist
/// pruning must be computed from these pre-rewrite sources, so compilation is
/// a separate pass from writing.
fn compile_island_chunks(
    islands: &[ClientIsland],
    dsc: Option<&Path>,
) -> Result<Vec<IslandChunk>, String> {
    let mut chunks = Vec::new();
    for directive in ["load", "idle", "visible"] {
        let group: Vec<&ClientIsland> = islands
            .iter()
            .filter(|island| island.directive == directive)
            .collect();
        if group.is_empty() {
            continue;
        }
        let mut seen_files = BTreeSet::new();
        let mut idx = 0usize;
        for island in &group {
            if !seen_files.insert(island.file.clone()) {
                continue;
            }
            let mut js = match dsc {
                Some(dsc) => crate::dsc_transpile::compile_file_with_dsc(&island.file, dsc),
                None => crate::dsc_transpile::compile_file(&island.file),
            }?;
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
            chunks.push(IslandChunk {
                directive,
                mod_stem,
                file: island.file.clone(),
                js,
                names: unique.iter().map(|name| (*name).to_string()).collect(),
            });
        }
    }
    Ok(chunks)
}

pub fn write_island_client_assets(
    assets_dir: &Path,
    islands: &[ClientIsland],
    flavor: ClientAssetFlavor,
) -> Result<(), String> {
    write_island_client_assets_with_optional_dsc(assets_dir, islands, flavor, None)
}

/// Build-only form with the compiler selected by the CLI. Artifact serve does
/// not call this and therefore never receives a compiler path.
pub fn write_island_client_assets_with_dsc(
    assets_dir: &Path,
    islands: &[ClientIsland],
    flavor: ClientAssetFlavor,
    dsc: &Path,
) -> Result<(), String> {
    write_island_client_assets_with_optional_dsc(assets_dir, islands, flavor, Some(dsc))
}

fn write_island_client_assets_with_optional_dsc(
    assets_dir: &Path,
    islands: &[ClientIsland],
    flavor: ClientAssetFlavor,
    dsc: Option<&Path>,
) -> Result<(), String> {
    let chunks = compile_island_chunks(islands, dsc)?;
    // Dist pruning needs the full importer set before any ui chunk is
    // written; dev skips the analysis entirely.
    let keep = if flavor == ClientAssetFlavor::Dist {
        let sources: Vec<String> = chunks.iter().map(|chunk| chunk.js.clone()).collect();
        compute_ui_keep_sets(&sources)?
    } else {
        None
    };
    let ui_names = write_ui_chunks(&assets_dir.join("ui"), flavor, &keep)?;
    let mut importmap_entries = ui_names.importmap_entries();
    let mut keep_root: BTreeSet<String> = BTreeSet::new();

    for directive in ["load", "idle", "visible"] {
        let group: Vec<&IslandChunk> = chunks
            .iter()
            .filter(|chunk| chunk.directive == directive)
            .collect();
        if group.is_empty() {
            continue;
        }
        let mut imports = String::new();
        let mut registers = String::new();
        for chunk in &group {
            let js = rewrite_ui_imports(&chunk.js, &ui_names);
            let js = optimize_client_chunk(&format!("{}.js", chunk.mod_stem), &js, flavor)?;
            let mod_name = hashed_asset_name(&chunk.mod_stem, "js", js.as_bytes());
            fs::write(assets_dir.join(&mod_name), js.as_bytes()).map_err(|err| {
                format!(
                    "failed to write {}: {err}",
                    assets_dir.join(&mod_name).display()
                )
            })?;
            keep_root.insert(mod_name.clone());
            imports.push_str(&format!(
                "import {{ {} }} from \"./{mod_name}\";\n",
                chunk.names.join(", ")
            ));
            for name in &chunk.names {
                registers.push_str(&format!(
                    "registerIsland({name_json}, {name});\n",
                    name_json =
                        serde_json::to_string(name).unwrap_or_else(|_| format!("\"{name}\"")),
                    name = name
                ));
            }
        }
        let entry = format!(
            "{imports}import {{ hydrate, registerIsland }} from \"./ui/{}\";\n{registers}hydrate();\n",
            ui_names.file("ui/client")
        );
        let entry = optimize_client_chunk(&format!("islands-{directive}.js"), &entry, flavor)?;
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

pub fn write_defer_client_assets(assets_dir: &Path, flavor: ClientAssetFlavor) -> Result<(), String> {
    let keep = if flavor == ClientAssetFlavor::Dist {
        compute_ui_keep_sets(&[])?
    } else {
        None
    };
    let ui_names = write_ui_chunks(&assets_dir.join("ui"), flavor, &keep)?;
    let entry = format!(
        "import {{ hydrate }} from \"./ui/{}\";\nhydrate();\n",
        ui_names.file("ui/client")
    );
    let entry = optimize_client_chunk("islands-defer.js", &entry, flavor)?;
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

/// Rewrite bare `from "ui/jsx"` (etc.) in compiled island JS to the hashed
/// relative path of the chunk that was actually written. The rewrite set is
/// `ui.hashed` — the same map `write_ui_chunks` returned — so a specifier
/// the writer skipped cannot appear here as a 404 (deka#622 finding E).
fn rewrite_ui_imports(js: &str, ui: &UiChunkNames) -> String {
    let mut out = js.to_string();
    for (spec, name) in &ui.hashed {
        let from = format!("from \"{spec}\"");
        if out.contains(&from) {
            out = out.replace(&from, &format!("from \"./ui/{name}\""));
        }
    }
    out
}

#[cfg(test)]
#[path = "islands_tests.rs"]
mod islands_tests;
