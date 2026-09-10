//! Per-route static render entries. Unlike the serve entry, these entries
//! import one page tree and expose StaticRender() with no Request argument.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::super::compiler_cache_dir;
use super::super::css::{CssPlan, collect_route_styles, css_plan_from_styles};
use super::super::defer::{defer_script_tag, enforce_defer_lints, scan_server_defer};
use super::super::document::CLIENT_IMPORTMAP_PLACEHOLDER_TAG;
use super::super::islands::{ClientIsland, island_script_tags, scan_client_islands};
use super::super::manifest::{FrameworkEntry, FrameworkEntryKind, FrameworkManifest, scan_app_dir};
use super::super::routes::{layout_chain, route_css_slug};
use super::defer::write_defer_router_entry;
use super::serve::{head_concat, split_document, with_css_links, wrap_layouts, wrap_layouts_with_params};
use super::{alias, exports_head, json_str, pathdiff_dsx};

/// Write a static entry for one route. `route` is the page's route TEMPLATE
/// (used to locate the page and its layout chain); `params` carries literal
/// values for the template's `[param]` segments. When params is non-empty the
/// entry is for one CONCRETE instance: the page call embeds the literal values
/// and the entry file is named after the concrete route.
pub fn write_static_render_entry(
    project_root: &Path,
    route: &str,
    params: &BTreeMap<String, String>,
) -> Result<PathBuf, String> {
    let app_dir = project_root.join("app");
    let manifest = scan_app_dir(&app_dir);
    validate_app_router(project_root, &manifest)?;
    let page = manifest
        .entries
        .iter()
        .find(|entry| entry.kind == FrameworkEntryKind::Page && entry.route == route)
        .ok_or_else(|| format!("no concrete app page for static route {route}"))?;

    enforce_defer_lints(&app_dir)?;
    let deferred = scan_server_defer(&app_dir);
    if !deferred.is_empty() {
        write_defer_router_entry(project_root)?;
    }
    let islands = scan_client_islands(&app_dir);
    let index_html = document_with_importmap(project_root, &islands, !deferred.is_empty())?;
    let mut scripts = island_script_tags(&islands);
    scripts.push_str(&defer_script_tag(!deferred.is_empty()));
    let styles = collect_route_styles(&manifest);
    let css_plan = css_plan_from_styles(&styles);

    let concrete_route = concrete_route(route, params);
    let cache_dir = compiler_cache_dir(project_root);
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join(format!(
        "static-{}-entry.dsx",
        route_css_slug(&concrete_route)
    ));
    let source = generate_static_entry(
        &entry,
        &manifest,
        page,
        &index_html,
        &scripts,
        &css_plan,
        params,
    )?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}

/// Substitute literal params into the template's `[param]` segments. The
/// manifest guarantees one value per bracket segment for staticParams
/// instances, so any leftover bracket means the caller passed a partial map.
fn concrete_route(template: &str, params: &BTreeMap<String, String>) -> String {
    let mut route = template.to_string();
    for (name, value) in params {
        route = route.replace(&format!("[{name}]"), value);
    }
    route
}

fn validate_app_router(project_root: &Path, manifest: &FrameworkManifest) -> Result<(), String> {
    if !manifest
        .entries
        .iter()
        .any(|entry| entry.kind == FrameworkEntryKind::Page && entry.route == "/")
    {
        return Err("app router requires app/page.dsx".to_string());
    }
    if !manifest
        .entries
        .iter()
        .any(|entry| entry.kind == FrameworkEntryKind::Layout && entry.route == "/")
    {
        return Err("app router requires app/layout.dsx (root layout)".to_string());
    }
    let index_path = project_root.join("index.html");
    if index_path.is_file() && project_root.join("public/index.html").is_file() {
        return Err("public/index.html collides with the root index.html document".to_string());
    }
    Ok(())
}

/// The document the static render fills: the project's root `index.html`
/// when present, otherwise the default harness (same posture as the serve
/// entry — `dist/client/index.html` is a build output, never a source
/// requirement).
fn document_with_importmap(
    project_root: &Path,
    islands: &[ClientIsland],
    has_deferred: bool,
) -> Result<String, String> {
    let index_html = super::serve::resolve_app_router_index_html(project_root)?;
    if (islands.is_empty() && !has_deferred) || index_html.contains("type=\"importmap\"") {
        return Ok(index_html);
    }
    let tag = CLIENT_IMPORTMAP_PLACEHOLDER_TAG;
    if index_html.contains("</head>") {
        Ok(index_html.replacen("</head>", &format!("  {tag}\n</head>"), 1))
    } else {
        Ok(format!("{tag}\n{index_html}"))
    }
}

fn generate_static_entry(
    entry: &Path,
    manifest: &FrameworkManifest,
    page: &FrameworkEntry,
    index_html: &str,
    scripts: &str,
    css_plan: &CssPlan,
    params: &BTreeMap<String, String>,
) -> Result<String, String> {
    let (doc_head, doc_mid, doc_tail) = split_document(index_html, scripts);
    let doc_head = json_str(&doc_head)?;
    let doc_mid = json_str(&doc_mid)?;
    let doc_tail = json_str(&doc_tail)?;

    let mut imports = String::new();
    let mut imported = Vec::new();
    let mut import_alias = |path: &str, name: &str, alias: &str| -> Result<(), String> {
        if imported.iter().any(|existing| existing == alias) {
            return Ok(());
        }
        imported.push(alias.to_string());
        let rel = json_str(&pathdiff_dsx(entry, Path::new(path)))?;
        imports.push_str(&format!("import {{ {name} as {alias} }} from {rel}\n"));
        Ok(())
    };

    let page_alias = alias("Page", &page.route);
    import_alias(&page.file, "Page", &page_alias)?;
    if exports_head(Path::new(&page.file)) {
        import_alias(&page.file, "head", &alias("head", &page.route))?;
    }
    for layout in layout_chain(&manifest.entries, &page.route) {
        import_alias(&layout.file, "Layout", &alias("Layout", &layout.route))?;
        if exports_head(Path::new(&layout.file)) {
            import_alias(&layout.file, "head", &alias("headL", &layout.route))?;
        }
    }

    let selected_loading: Vec<&FrameworkEntry> = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == FrameworkEntryKind::Loading)
        .filter(|entry| route_is_within(&page.route, &entry.route))
        .collect();
    for loading in &selected_loading {
        import_alias(&loading.file, "Loading", &alias("Loading", &loading.route))?;
    }
    let suspense_import = if selected_loading.is_empty() {
        ""
    } else {
        "import { Suspense } from \"ui/suspense\"\n"
    };

    let tree = if params.is_empty() {
        wrap_layouts(&manifest.entries, &page.route, &page_alias)
    } else {
        wrap_layouts_with_params(&manifest.entries, &page.route, &page_alias, params)?
    };
    let head = with_css_links(
        css_plan,
        &page.route,
        head_concat(&manifest.entries, page, false),
    )?;
    Ok(format!(
        r#"{suspense_import}{imports}
fn head_html(node: Component) string {{
    const result = unsafe {{ deka.ui.renderToString(node) }}
    return match (result) {{
        Ok(rendered) => rendered.html,
        Err(_) => "",
    }}
}}

async fn static_html(tree: Component) Promise<string> {{
    const boxed = unsafe {{ deka.ui.renderToStringAsync(tree) }}
    const prom = match (boxed) {{
        Ok(p) => p,
        Err(_) => {{ html: "" }},
        _ => {{ html: "" }},
    }}
    const rendered = await prom
    const html = unsafe {{ rendered.html }}
    return match (html) {{
        Ok(value) => value,
        Err(_) => "",
        _ => "",
    }}
}}

export async fn StaticRender() {{
    const tree = {tree}
    const head = {head}
    const appHtml = await static_html(tree)
    return {{ status: 200, headers: {{}}, body: {doc_head} + head + {doc_mid} + appHtml + {doc_tail} }}
}}
"#
    ))
}

fn route_is_within(route: &str, segment: &str) -> bool {
    segment == "/"
        || route == segment
        || route.starts_with(&format!("{}/", segment.trim_end_matches('/')))
}
