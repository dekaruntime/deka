//! Build-time server-entry emission into `dist/server/` (deka#762, phase 2 of
//! deka#743).
//!
//! `deka serve` used to generate `serve-entry.dsx` into the compiler cache at
//! startup and recompile the whole app graph from `.ds`/`.dsx` source — the
//! built artifact was not what ran (deka#743). This module moves that work to
//! `deka build`:
//!
//! 1. Generate the same router entries serve-time generation produces
//!    (pages, api, defer) into an entries dir inside the project tree, two
//!    levels deep, so their relative imports resolve against project sources
//!    exactly as the cache entries do.
//! 2. Compile each entry through dsc's self-contained graph dump.
//! 3. Re-root the dumped graph under `dist/server/`: every module lands at
//!    its source-tree-relative `.js` path, and every relative specifier is
//!    recomputed between the new locations so the graph stays closed (§4.2:
//!    extension-explicit relative specifiers resolving inside the server
//!    root — the emitted artifact carries no `.ds`/`.dsx` and no absolute
//!    path).
//! 4. Make build values ordinary modules at `server/.values/<id>.js` (the
//!    `deka:dev/<id>` virtual scheme is rewritten away everywhere, client
//!    bundles included, and any survivor fails the build).
//!
//! The output is the manifest-v2 layout (§1): every executable module under
//! `dist/server/`, described with digests in `dist/build-manifest.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use runtime_core::dist::{
    PlannedSource, generate_api_entry_source, generate_app_router_entry_source,
    generate_defer_entry_source, resolve_app_router_index_html, scan_api_dir, scan_server_defer,
};

/// Where materialized build-value modules live inside the server tree.
pub const VALUES_DIR: &str = ".values";
/// Where the vendored ui runtime modules live inside the server tree.
pub const UI_DIR: &str = ".ui";

/// The compiled router entries, as `dist/`-relative module paths.
#[derive(Debug, Default)]
pub struct EmittedEntries {
    pub serve: Option<String>,
    pub api: Option<String>,
    pub defer: Option<String>,
}

/// Generate the app-router router entries into `entries_dir`, compile each
/// through dsc, and re-root the merged module graph into `dist_server`.
///
/// `entries_dir` must sit two levels below the project root (e.g.
/// `.deka-dist-stage/entries`): the generated entries import project sources
/// by `../../app/...`-style relative paths, so the entry's on-disk location
/// determines what dsc resolves. Returns the dist-relative paths of the
/// compiled router entries for the manifest's server entry table.
#[cfg(feature = "native")]
pub fn compile_and_reroot_entries(
    project_root: &Path,
    entries_dir: &Path,
    dist_server: &Path,
    dsc: &Path,
) -> Result<EmittedEntries, String> {
    let sources = generate_entry_sources(project_root, entries_dir)?;

    let mut modules: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut emitted = EmittedEntries::default();
    // Sources are ordered serve, api, defer: the FIRST compile to emit a
    // module wins. The graphs legitimately differ per entry — dsc tree-shakes
    // each graph to what the entry imports (e.g. under the defer entry a page
    // module loses its `Page` export and keeps only the deferred component),
    // so byte-identical merging is wrong. The serve graph is the superset for
    // route modules; correctness of whichever variant won is enforced after
    // the write by `assert_entry_linkage` — every named import of every
    // emitted entry must resolve in the merged graph.
    for (name, source_path) in &sources {
        let graph = pool::dsc_compile::compile_graph_with_dsc(project_root, source_path, dsc)
            .map_err(|err| format!("failed to compile server entry {name}: {err}"))?;
        for (path, js) in graph {
            modules.entry(path).or_insert(js);
        }
    }

    // Map every dumped module (keyed by canonical source path) to its new
    // home inside dist/server.
    let entries_dir = fs::canonicalize(entries_dir).unwrap_or_else(|_| entries_dir.to_path_buf());
    let project_root =
        fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    let mut targets: BTreeMap<PathBuf, String> = BTreeMap::new();
    for source in modules.keys() {
        let target = reroot_target(&project_root, &entries_dir, source).ok_or_else(|| {
            format!(
                "dsc graph module {} is outside the project; cannot re-root into dist/server",
                source.display()
            )
        })?;
        targets.insert(source.clone(), target);
    }
    for (name, _) in &sources {
        emitted_set(&mut emitted, name, &targets, &entries_dir)?;
    }

    // Rewrite every specifier between the new locations and write the files.
    for (source, js) in &modules {
        let importer_target = targets.get(source).expect("target computed above");
        let rewritten = rewrite_module_specifiers(
            js,
            source,
            &targets,
            &project_root,
            importer_target,
        )?;
        let dest = dist_server.join(importer_target);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        fs::write(&dest, rewritten.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", dest.display()))?;
    }

    assert_entry_linkage(dist_server, &emitted)?;
    Ok(emitted)
}

/// Every named import of every emitted router entry must resolve in the
/// merged graph. Per-entry graphs are tree-shaken, so when two entries emit
/// differing bytes for one source module the first (serve) variant wins; this
/// check is the guard that the winning variant still satisfies the other
/// entries — a dangling or unexported binding is a build error, never a
/// silently broken artifact.
fn assert_entry_linkage(dist_server: &Path, emitted: &EmittedEntries) -> Result<(), String> {
    for module in [&emitted.serve, &emitted.api, &emitted.defer]
        .into_iter()
        .flatten()
    {
        let rel = module
            .strip_prefix("server/")
            .unwrap_or(module);
        let entry_path = dist_server.join(rel);
        let js = fs::read_to_string(&entry_path)
            .map_err(|err| format!("failed to read {}: {err}", entry_path.display()))?;
        for (specifier, names) in named_imports(&js) {
            // Bare specifiers are left for the loader's module resolution
            // (paused-framework `ui/*` imports among them); only relative
            // specifiers are part of the merged-graph linkage check, same
            // jail rule as `assert_server_jail` below.
            if !(specifier.starts_with("./") || specifier.starts_with("../")) {
                continue;
            }
            let resolved = normalize_path(
                &entry_path
                    .parent()
                    .unwrap_or(dist_server)
                    .join(&specifier),
            );
            if !resolved.starts_with(dist_server) {
                return Err(format!(
                    "server entry {rel} imports `{specifier}`, which escapes dist/server"
                ));
            }
            let target = fs::read_to_string(&resolved).map_err(|err| {
                format!(
                    "server entry {rel} imports `{specifier}` -> {}: {err}",
                    resolved.display()
                )
            })?;
            for name in names {
                if !has_export(&target, &name) {
                    return Err(format!(
                        "server entry {rel} imports `{name}` from `{specifier}`, but the \
                         merged module {} does not export it — the per-entry compile \
                         variants could not be reconciled",
                        resolved.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// `(specifier, imported names)` for each `import { .. } from ".."` statement
/// in dsc-emitted JS (single statements, possibly multiline brace lists).
fn named_imports(js: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let bytes = js.as_bytes();
    let mut cursor = 0;
    while let Some(found) = js[cursor..].find("import") {
        let start = cursor + found;
        // Only statement-position `import` (start of line or after `;`/`}` +
        // whitespace) — skip substrings inside identifiers like
        // "important".
        let before_ok = start == 0
            || matches!(bytes[start - 1], b';' | b'\n' | b'}' | b' ');
        let after = start + "import".len();
        let after_ok = bytes
            .get(after)
            .is_some_and(|byte| byte.is_ascii_whitespace());
        if !(before_ok && after_ok) {
            cursor = after;
            continue;
        }
        let Some(open) = js[after..].find('{') else {
            cursor = after;
            continue;
        };
        let open = after + open;
        let Some(close) = js[open..].find('}') else {
            cursor = after;
            continue;
        };
        let close = open + close;
        let Some(from_at) = js[close..].find("from") else {
            cursor = close + 1;
            continue;
        };
        let from_at = close + from_at;
        let quote_at = from_at + "from".len();
        let quote = match js.as_bytes().get(quote_at + 1) {
            Some(b'"') | Some(b'\'') => quote_at + 1,
            _ => {
                cursor = close + 1;
                continue;
            }
        };
        let Some(end) = js[quote + 1..].find(js.as_bytes()[quote] as char) else {
            cursor = close + 1;
            continue;
        };
        let specifier = js[quote + 1..quote + 1 + end].to_string();
        let names = js[open + 1..close]
            .split(',')
            .filter_map(|part| {
                let part = part.trim();
                // `X as Y` binds the module's export `X` locally as `Y`; the
                // export the target module must provide is the LEFT side.
                part.split(" as ").next().map(str::trim).filter(|name| {
                    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                }).map(str::to_string)
            })
            .collect();
        out.push((specifier, names));
        cursor = quote + 1 + end;
    }
    out
}

/// Does this module export `name`? Covers the emission shapes dsc produces:
/// `export function/const/class/let/var/async function NAME` and
/// `export { A, B as C }` lists (plus `export default` for `default`).
fn has_export(js: &str, name: &str) -> bool {
    if name == "default" {
        return js.contains("export default");
    }
    for kw in ["function", "const", "class", "let", "var"] {
        if js.contains(&format!("export {kw} {name}")
            ) || js.contains(&format!("export async {kw} {name}"))
        {
            return true;
        }
    }
    let mut cursor = 0;
    while let Some(found) = js[cursor..].find("export {") {
        let at = cursor + found;
        let open = at + js[at..].find('{').expect("brace follows `export `");
        let Some(close) = js[open..].find('}') else { break };
        let close = open + close;
        for item in js[open + 1..close].split(',') {
            // In an export list the RIGHT side of `as` is the exported name
            // (opposite of an import list).
            let exported = item.trim().rsplit(" as ").next().unwrap_or("").trim();
            if exported == name {
                return true;
            }
        }
        cursor = close + 1;
    }
    false
}

fn emitted_set(
    emitted: &mut EmittedEntries,
    name: &str,
    targets: &BTreeMap<PathBuf, String>,
    entries_dir: &Path,
) -> Result<(), String> {
    let source = fs::canonicalize(entries_dir.join(name)).unwrap_or_else(|_| entries_dir.join(name));
    let target = targets.get(&source).ok_or_else(|| {
        format!(
            "compiled server entry {name} is missing from the dsc graph dump"
        )
    })?;
    let module = format!("server/{target}");
    match name {
        "serve-entry.dsx" => emitted.serve = Some(module),
        "api-entry.ds" => emitted.api = Some(module),
        "defer-entry.dsx" => emitted.defer = Some(module),
        other => return Err(format!("unknown server entry {other}")),
    }
    Ok(())
}

/// Generate the router entry sources (without compiling). Shared with
/// [`compile_and_reroot_entries`] so tests can exercise generation alone.
#[cfg(feature = "native")]
fn generate_entry_sources(
    project_root: &Path,
    entries_dir: &Path,
) -> Result<Vec<(String, PathBuf)>, String> {
    let index_html = resolve_app_router_index_html(project_root)?;
    fs::create_dir_all(entries_dir)
        .map_err(|err| format!("failed to create {}: {err}", entries_dir.display()))?;

    let serve_path = entries_dir.join("serve-entry.dsx");
    let serve = generate_app_router_entry_source(&serve_path, project_root, &index_html)?;
    let mut sources = vec![("serve-entry.dsx".to_string(), serve_path.clone())];
    fs::write(&serve_path, serve.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", serve_path.display()))?;

    let api_entries = scan_api_dir(&project_root.join("api"));
    if !api_entries.is_empty() {
        let path = entries_dir.join("api-entry.ds");
        let source = generate_api_entry_source(&path, &api_entries)?;
        fs::write(&path, source.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
        sources.push(("api-entry.ds".to_string(), path));
    }

    let deferred = scan_server_defer(&project_root.join("app"));
    if !deferred.is_empty() {
        let path = entries_dir.join("defer-entry.dsx");
        let source = generate_defer_entry_source(&path, project_root, &deferred)?;
        fs::write(&path, source.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
        sources.push(("defer-entry.dsx".to_string(), path));
    }
    Ok(sources)
}

/// Where a dumped graph module lives inside `dist/server`: generated entries
/// keep their file name (extension swapped), project sources keep their
/// tree-relative path with `.ds`/`.dsx` swapped for `.js`.
fn reroot_target(project_root: &Path, entries_dir: &Path, source: &Path) -> Option<String> {
    if let Ok(rel) = source.strip_prefix(entries_dir) {
        return Some(swap_source_ext(rel));
    }
    let rel = source.strip_prefix(project_root).ok()?;
    Some(swap_source_ext(rel))
}

fn swap_source_ext(rel: &Path) -> String {
    let text = rel.to_string_lossy().replace('\\', "/");
    let stem = text
        .strip_suffix(".dsx")
        .or_else(|| text.strip_suffix(".ds"))
        .unwrap_or(&text);
    format!("{stem}.js")
}

/// Rewrite every import/export specifier in one dumped module so it resolves
/// between the module's new location and its targets':
///
/// - relative specifiers are recomputed against the re-rooted target paths
///   (the dumped graph may still spell peer sources `.ds`/`.dsx`; the
///   artifact only ever spells `.js`);
/// - `deka:dev/<id>` becomes a relative path into `server/.values/<id>.js`;
/// - bare `ui/<name>` specifiers are paused-framework imports: left
///   unrewritten here, and skipped by every resolution check below;
/// - any other bare specifier is left for the loader's module resolution
///   (the artifact ships `server/ds_modules`, which resolves them).
#[allow(clippy::too_many_arguments)]
fn rewrite_module_specifiers(
    js: &str,
    source: &Path,
    targets: &BTreeMap<PathBuf, String>,
    project_root: &Path,
    importer_target: &str,
) -> Result<String, String> {
    let mut out = js.to_string();
    let specs = runtime_core::ds_imports::paths(js);
    for spec in specs {
        let replacement = match classify_specifier(&spec) {
            SpecKind::Relative => {
                let resolved = normalize_path(
                    &source
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(&spec),
                );
                let target = find_target(&resolved, targets).ok_or_else(|| {
                    format!(
                        "server graph module {} imports `{}`, which is not part of the \
                         compiled graph; the artifact would ship a dangling specifier",
                        source.display(),
                        spec
                    )
                })?;
                let importer_dir = Path::new(importer_target)
                    .parent()
                    .unwrap_or_else(|| Path::new(""));
                Some(relative_specifier(
                    importer_dir,
                    Path::new(&target),
                ))
            }
            SpecKind::BuildValue(id) => {
                let values_target = format!("{VALUES_DIR}/{id}.js");
                let importer_dir = Path::new(importer_target)
                    .parent()
                    .unwrap_or_else(|| Path::new(""));
                Some(relative_specifier(importer_dir, Path::new(&values_target)))
            }
            SpecKind::Bare => {
                let _ = project_root;
                None
            }
        };
        if let Some(replacement) = replacement {
            out = replace_quoted(&out, &spec, &replacement);
        }
    }
    Ok(out)
}

enum SpecKind {
    Relative,
    BuildValue(String),
    Bare,
}

fn classify_specifier(spec: &str) -> SpecKind {
    if spec.starts_with("./") || spec.starts_with("../") {
        return SpecKind::Relative;
    }
    if let Some(id) = spec.strip_prefix("deka:dev/") {
        if !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return SpecKind::BuildValue(id.to_string());
        }
        return SpecKind::Bare;
    }
    SpecKind::Bare
}

/// Find the re-rooted target for a resolved (normalized) source path the
/// importer referenced. The dumped graph is keyed by source paths; dsc may
/// spell the specifier with the original `.ds`/`.dsx` extension or the
/// compiled `.js` one, so try the neighbours.
fn find_target(resolved: &Path, targets: &BTreeMap<PathBuf, String>) -> Option<String> {
    let text = resolved.to_string_lossy();
    let mut candidates: Vec<PathBuf> = vec![resolved.to_path_buf()];
    if let Some(stem) = text.strip_suffix(".js") {
        candidates.push(PathBuf::from(format!("{stem}.dsx")));
        candidates.push(PathBuf::from(format!("{stem}.ds")));
    }
    for candidate in &candidates {
        if let Some(target) = targets.get(candidate) {
            return Some(target.clone());
        }
    }
    None
}

/// Lexically normalize `.` / `..` segments (no filesystem access; the paths
/// come from dsc's own dump keys, which are already canonical).
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `./`-prefixed forward-slash relative path from `from_dir` (relative to
/// the server root) to `to` (also relative to the server root).
pub fn relative_specifier(from_dir: &Path, to: &Path) -> String {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to.components().collect();
    let mut shared = 0;
    while shared < from.len() && shared < to.len() && from[shared] == to[shared] {
        shared += 1;
    }
    let mut parts: Vec<String> = (shared..from.len()).map(|_| "..".to_string()).collect();
    parts.extend(to[shared..].iter().map(|component| {
        component.as_os_str().to_string_lossy().into_owned()
    }));
    let spec = if parts.is_empty() {
        "./".to_string()
    } else {
        parts.join("/")
    };
    // ESM relative specifiers must start with `./` or `../`; anything else
    // (including dot-directory names like `.ui/x.js`) would be re-read as a
    // package specifier.
    if spec.starts_with("./") || spec.starts_with("../") {
        spec
    } else {
        format!("./{spec}")
    }
}

/// Replace both quoted spellings of `from` with `to`. `ds_imports::paths`
/// already established these are real import/export specifiers, so the
/// quoted-needle swap (the same approach the build-value rewrite has always
/// used) cannot touch unrelated code.
fn replace_quoted(source: &str, from: &str, to: &str) -> String {
    let mut out = source.replace(&format!("\"{from}\""), &format!("\"{to}\""));
    out = out.replace(&format!("'{from}'"), &format!("'{to}'"));
    out
}

/// Copy this build's materialized build-value modules into
/// `server/.values/`, rewrite every `deka:dev/<id>` specifier across the
/// whole dist tree (client bundles included) to a relative path into them,
/// and fail the build loudly if any dev-scheme specifier survives: the
/// staged tree is then not deployable and must not publish. Bare `ui/*`
/// specifiers are paused-framework imports and are intentionally left
/// unrewritten.
#[cfg(feature = "native")]
pub fn publish_build_values(
    project_root: &Path,
    dist_root: &Path,
    dist_server: &Path,
    planned: &[PlannedSource],
) -> Result<(), String> {
    let ids: BTreeSet<String> = planned
        .iter()
        .flat_map(|source| source.plan.slots.iter().map(|slot| slot.id.clone()))
        .collect();
    if !ids.is_empty() {
        copy_build_value_modules(
            &runtime_core::dist::compiler_cache_dir(project_root).join("build-values"),
            dist_server,
            &ids,
        )?;
    }
    rewrite_build_value_specifiers(dist_root, dist_server, &ids)?;
    assert_server_jail(dist_server)?;
    Ok(())
}

/// Copy the materialized build-value modules for `ids` from the project
/// cache into `<dist_server>/.values/`.
pub fn copy_build_value_modules(
    cache_values_dir: &Path,
    dist_server: &Path,
    ids: &BTreeSet<String>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let target_dir = dist_server.join(VALUES_DIR);
    fs::create_dir_all(&target_dir)
        .map_err(|err| format!("failed to create {}: {err}", target_dir.display()))?;
    for id in ids {
        let source = cache_values_dir.join(format!("{id}.js"));
        if !source.is_file() {
            return Err(format!(
                "build value `{}` was not materialized at {}",
                id,
                source.display()
            ));
        }
        fs::copy(&source, target_dir.join(format!("{id}.js"))).map_err(|err| {
            format!(
                "failed to copy {} -> {}: {err}",
                source.display(),
                target_dir.join(format!("{id}.js")).display()
            )
        })?;
    }
    Ok(())
}

/// Rewrite every `"deka:dev/<id>"` literal (quotes included) in all `.js`
/// files under `dist_root` to the relative path from the importing file to
/// `<dist_server>/.values/<id>.js`. Fails the build loudly if any
/// `deka:dev/` specifier survives.
fn rewrite_build_value_specifiers(
    dist_root: &Path,
    dist_server: &Path,
    ids: &BTreeSet<String>,
) -> Result<(), String> {
    let mut js_files = Vec::new();
    collect_js_files(dist_root, &mut js_files)?;
    for file in &js_files {
        let mut source = fs::read_to_string(file)
            .map_err(|err| format!("failed to read {}: {err}", file.display()))?;
        if !source.contains("deka:dev/") {
            continue;
        }
        let importer_dir = file.parent().unwrap_or(dist_root);
        for id in ids {
            for quote in ['"', '\''] {
                let needle = format!("{quote}deka:dev/{id}{quote}");
                if !source.contains(&needle) {
                    continue;
                }
                let target = dist_server.join(VALUES_DIR).join(format!("{id}.js"));
                let rewritten = format!("{quote}{}{quote}", relative_path(importer_dir, &target));
                source = source.replace(&needle, &rewritten);
            }
        }
        fs::write(file, source)
            .map_err(|err| format!("failed to write {}: {err}", file.display()))?;
    }
    assert_no_dev_specifiers(dist_root)
}

/// §4.2 jail: every relative specifier in every server module must resolve,
/// lexically, to a file inside `dist_server`. Anything else is a build error —
/// the artifact must not carry a path that escapes its execution root.
fn assert_server_jail(dist_server: &Path) -> Result<(), String> {
    let mut js_files = Vec::new();
    collect_js_files(dist_server, &mut js_files)?;
    for file in &js_files {
        let source = fs::read_to_string(file)
            .map_err(|err| format!("failed to read {}: {err}", file.display()))?;
        for spec in runtime_core::ds_imports::paths(&source) {
            if !(spec.starts_with("./") || spec.starts_with("../")) {
                continue;
            }
            let resolved = normalize_path(&file.parent().unwrap_or(dist_server).join(&spec));
            if !resolved.starts_with(dist_server) {
                return Err(format!(
                    "server module {} imports `{}`, which resolves outside dist/server",
                    file.display(),
                    spec
                ));
            }
            if !resolved.is_file() {
                return Err(format!(
                    "server module {} imports `{}`, which does not resolve to a file \
                     inside dist/server",
                    file.display(),
                    spec
                ));
            }
        }
    }
    Ok(())
}

/// A dev-scheme specifier left anywhere in the staged dist tree means the
/// output is not deployable — fail the build instead of publishing it.
fn assert_no_dev_specifiers(dist_root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect_files(dist_root, &mut files)?;
    for file in files {
        let bytes =
            fs::read(&file).map_err(|err| format!("failed to read {}: {err}", file.display()))?;
        if bytes.windows(b"deka:dev/".len()).any(|w| w == b"deka:dev/") {
            return Err(format!(
                "build output is not self-contained: `{}` still references a deka:dev/ specifier",
                file.display()
            ));
        }
    }
    Ok(())
}

fn collect_js_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    collect_files(dir, out)?;
    out.retain(|path| {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("js"))
    });
    Ok(())
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Forward-slash relative path from `from_dir` to `to_file`, `./`-prefixed
/// when the file sits in the same directory. Both must sit under the same
/// tree (the staged dist root and its files do).
pub fn relative_path(from_dir: &Path, to_file: &Path) -> String {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to_file.components().collect();
    let mut shared = 0;
    while shared < from.len() && shared < to.len() && from[shared] == to[shared] {
        shared += 1;
    }
    let mut parts: Vec<String> = (shared..from.len()).map(|_| "..".to_string()).collect();
    parts.extend(to[shared..].iter().map(|component| {
        component.as_os_str().to_string_lossy().into_owned()
    }));
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
#[path = "build_server_graph_tests.rs"]
mod tests;
