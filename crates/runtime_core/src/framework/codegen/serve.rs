//! `serve-entry.dsx`: routes `request.pathname` through the `app/` matcher
//! (nested layouts, `loading.dsx` → `Suspense`, not-found, fragment Accept,
//! per-route CSS links).

use std::path::{Path, PathBuf};

use super::super::css::{CssPlan, collect_route_styles, css_links_for_route, css_plan_from_styles};
use super::super::defer::{defer_script_tag, enforce_defer_lints, scan_server_defer};
use super::super::document::{
    CLIENT_IMPORTMAP_PLACEHOLDER_TAG, DEKA_APP_HOLE, DEKA_HEAD_HOLE, DEKA_SCRIPTS_HOLE,
    FRAGMENT_ACCEPT, FRAGMENT_ACCEPT_LEGACY, STATIC_ACCEPT,
};
use super::super::islands::{island_script_tags, scan_client_islands};
use super::super::manifest::{
    FrameworkEntry, FrameworkEntryKind, FrameworkManifest, scan_api_dir, scan_app_dir,
};
use super::super::routes::{
    ancestor_routes, assert_dynamic_route_supported, dynamic_param_names, dynamic_rank,
    layout_chain, static_prefix,
};
use super::api::write_api_router_entry;
use super::defer::write_defer_router_entry;
use super::{
    alias, ensure_defer_secret, exports_head, json_str, pathdiff_dsx, session_cookie_name,
};
/// Write a generated App() entry that routes `request.pathname` through the
/// `app/` matcher (nested layouts, not-found, fragment Accept).
pub fn write_app_router_entry(project_root: &Path) -> Result<PathBuf, String> {
    let app_dir = project_root.join("app");
    let manifest = scan_app_dir(&app_dir);
    if !manifest
        .entries
        .iter()
        .any(|e| e.kind == FrameworkEntryKind::Page && e.route == "/")
    {
        return Err("app router requires app/page.dsx".to_string());
    }
    if !manifest
        .entries
        .iter()
        .any(|e| e.kind == FrameworkEntryKind::Layout && e.route == "/")
    {
        return Err("app router requires app/layout.dsx (root layout)".to_string());
    }
    let index_path = project_root.join("index.html");
    if !index_path.is_file() {
        return Err("app router requires index.html at the project root".to_string());
    }
    if project_root.join("public/index.html").is_file() {
        return Err("public/index.html collides with the root index.html document".to_string());
    }
    let index_html = std::fs::read_to_string(&index_path)
        .map_err(|err| format!("failed to read {}: {err}", index_path.display()))?;
    let islands = scan_client_islands(&app_dir);
    enforce_defer_lints(&app_dir)?;
    let deferred = scan_server_defer(&app_dir);
    // RFD 24 §10.7: a document that loads client chunks resolves bare
    // specifiers through an inline import map. The map does not exist yet at
    // generation time — the hashes are assigned when the client assets are
    // written, after this entry is generated — so bake the placeholder tag and
    // let `runtime::islands::rewrite_serve_entry_asset_urls` swap in the
    // inlined JSON once the assets exist. `deka build` inlines the same map
    // into dist HTML only when the prerendered document lacks one.
    let index_html = if (!islands.is_empty() || !deferred.is_empty())
        && !index_html.contains("type=\"importmap\"")
    {
        let tag = CLIENT_IMPORTMAP_PLACEHOLDER_TAG;
        if index_html.contains("</head>") {
            index_html.replacen("</head>", &format!("  {tag}\n</head>"), 1)
        } else {
            format!("{tag}\n{index_html}")
        }
    } else {
        index_html
    };
    let mut scripts = island_script_tags(&islands);
    scripts.push_str(&defer_script_tag(!deferred.is_empty()));
    if !deferred.is_empty() {
        write_defer_router_entry(project_root)?;
    }
    let styles = collect_route_styles(&manifest);
    let css_plan = css_plan_from_styles(&styles);
    let cache_dir = project_root.join(".cache").join("dekascript");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("serve-entry.dsx");
    let defer_secret_js = if deferred.is_empty() {
        String::new()
    } else {
        let secret = json_str(&ensure_defer_secret(project_root)?)?;
        let cookie = json_str(&session_cookie_name(project_root))?;
        format!("    unsafe {{ deka.ui.bindDefer(request, {secret}, {cookie}) }}\n")
    };
    let source = generate_serve_entry(
        &entry,
        &manifest,
        &index_html,
        &scripts,
        &css_plan,
        &defer_secret_js,
    )?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    if !scan_api_dir(&project_root.join("api")).is_empty() {
        write_api_router_entry(project_root)?;
    }
    Ok(entry)
}
fn generate_serve_entry(
    entry: &Path,
    manifest: &FrameworkManifest,
    index_html: &str,
    scripts: &str,
    css_plan: &CssPlan,
    defer_secret_js: &str,
) -> Result<String, String> {
    let (doc_head, doc_mid, doc_tail) = split_document(index_html, scripts);
    let doc_head = json_str(&doc_head)?;
    let doc_mid = json_str(&doc_mid)?;
    let doc_tail = json_str(&doc_tail)?;

    let mut imports = String::new();
    let mut imported: Vec<String> = Vec::new();
    let mut import_alias = |path: &str, name: &str, alias: &str| -> Result<(), String> {
        if imported.iter().any(|k| k == alias) {
            return Ok(());
        }
        imported.push(alias.to_string());
        let rel = json_str(&pathdiff_dsx(entry, Path::new(path)))?;
        imports.push_str(&format!("import {{ {name} as {alias} }} from {rel}\n"));
        Ok(())
    };

    let mut pages: Vec<&FrameworkEntry> = manifest
        .entries
        .iter()
        .filter(|e| e.kind == FrameworkEntryKind::Page)
        .collect();
    pages.sort_by_key(|e| dynamic_rank(&e.route));

    for page in &pages {
        import_alias(&page.file, "Page", &alias("Page", &page.route))?;
        if exports_head(Path::new(&page.file)) {
            import_alias(&page.file, "head", &alias("head", &page.route))?;
        }
    }
    for layout in manifest
        .entries
        .iter()
        .filter(|e| e.kind == FrameworkEntryKind::Layout)
    {
        import_alias(&layout.file, "Layout", &alias("Layout", &layout.route))?;
        if exports_head(Path::new(&layout.file)) {
            import_alias(&layout.file, "head", &alias("headL", &layout.route))?;
        }
    }
    let has_loading = manifest
        .entries
        .iter()
        .any(|e| e.kind == FrameworkEntryKind::Loading);
    for loading in manifest
        .entries
        .iter()
        .filter(|e| e.kind == FrameworkEntryKind::Loading)
    {
        import_alias(&loading.file, "Loading", &alias("Loading", &loading.route))?;
    }
    if let Some(not_found) = &manifest.not_found {
        import_alias(&not_found.file, "Page", "Page_not_found")?;
        if exports_head(Path::new(&not_found.file)) {
            import_alias(&not_found.file, "head", "head_not_found")?;
        }
    }

    let mut branches = String::new();
    for page in &pages {
        let cond = path_condition(&page.route)?;
        let tree = wrap_layouts(&manifest.entries, &page.route, &alias("Page", &page.route));
        let head = with_css_links(
            css_plan,
            &page.route,
            head_concat(&manifest.entries, page, false),
        )?;
        branches.push_str(&format!(
            "    if ({cond}) {{\n        return await respond({tree}, 200, fragment, staticBuild, {head})\n    }}\n"
        ));
    }
    let not_found_tree = if manifest.not_found.is_some() {
        wrap_layouts(&manifest.entries, "/", "Page_not_found")
    } else {
        "FallbackNotFound()".to_string()
    };
    let not_found_route = if manifest.not_found.is_some() {
        "__not_found"
    } else {
        "/"
    };
    let not_found_head_inner = if manifest
        .not_found
        .as_ref()
        .is_some_and(|e| exports_head(Path::new(&e.file)))
    {
        head_expr(
            &layout_head_aliases(&manifest.entries, "/"),
            "head_not_found()",
        )
    } else {
        head_expr(&layout_head_aliases(&manifest.entries, "/"), "\"\"")
    };
    let not_found_head = with_css_links(css_plan, not_found_route, not_found_head_inner)?;

    let fallback_fn = if manifest.not_found.is_some() {
        String::new()
    } else {
        "fn FallbackNotFound() {\n    return <section><h1>Not found</h1></section>;\n}\n\n"
            .to_string()
    };
    let suspense_import = if has_loading {
        "import { Suspense } from \"ui/suspense\"\n"
    } else {
        ""
    };

    Ok(format!(
        r#"{suspense_import}{imports}
interface RequestHeaders {{ accept: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders }}
interface Response {{ status: number, body: string }}

{fallback_fn}fn starts_with(s: string, prefix: string) boolean {{
    let i = 0
    for (const want of prefix) {{
        let j = 0
        let got = ""
        for (const ch of s) {{
            if (j == i) {{ got = ch }}
            j = j + 1
        }}
        if (got != want) {{ return false }}
        i = i + 1
    }}
    return true
}}

fn one_segment_after(s: string, prefix: string) boolean {{
    if (!starts_with(s, prefix)) {{ return false }}
    let n = 0
    for (const _ of prefix) {{ n = n + 1 }}
    let rest = 0
    let slashes = 0
    let i = 0
    for (const ch of s) {{
        if (i >= n) {{
            rest = rest + 1
            if (ch == "/") {{ slashes = slashes + 1 }}
        }}
        i = i + 1
    }}
    return rest > 0 && slashes == 0
}}

fn last_segment(s: string) string {{
    let last = ""
    let cur = ""
    for (const ch of s) {{
        if (ch == "/") {{
            cur = ""
        }} else {{
            cur = cur + ch
            last = cur
        }}
    }}
    return last
}}

fn head_html(node: Component) string {{
    const result = unsafe {{ deka.ui.renderToString(node) }}
    return match (result) {{
        Ok(rendered) => rendered.html,
        Err(_) => "",
    }}
}}

fn title_from_head(headHtml: string) string {{
    let result = unsafe {{
        var openAt = headHtml.indexOf("<title>")
        if (openAt < 0) {{ return "" }}
        var closeAt = headHtml.indexOf("</title>", openAt + 7)
        if (closeAt < 0) {{ return "" }}
        return headHtml.slice(openAt + 7, closeAt)
    }}
    return match (result) {{
        Ok(title) => title,
        Err(_) => "",
    }}
}}

async fn stream_html(tree: Component) Promise<string> {{
    const boxed = unsafe {{ deka.ui.renderToStreamHtml(tree) }}
    const prom = match (boxed) {{
        Ok(p) => p,
        Err(_) => "",
        _ => "",
    }}
    return await prom
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
        Ok(h) => h,
        Err(_) => "",
        _ => "",
    }}
}}

async fn respond(tree: Component, status: number, fragment: boolean, staticBuild: boolean, headHtml: string) Promise<Response> {{
    if (fragment) {{
        const result = unsafe {{ deka.ui.renderToString(tree) }}
        const appHtml = match (result) {{
            Ok(rendered) => rendered.html,
            Err(_) => "<p>Internal Server Error</p>",
        }}
        const payload = unsafe {{ JSON.stringify({{ html: appHtml, title: title_from_head(headHtml), head: headHtml }}) }}
        return match (payload) {{
            Ok(json) => {{ status: status, body: json }},
            Err(_) => {{ status: 500, body: "Internal Server Error" }},
        }}
    }}
    if (staticBuild) {{
        const appHtml = await static_html(tree)
        return {{ status: status, body: {doc_head} + headHtml + {doc_mid} + appHtml + {doc_tail} }}
    }}
    const appHtml = await stream_html(tree)
    return {{ status: status, body: {doc_head} + headHtml + {doc_mid} + appHtml + {doc_tail} }}
}}

async fn App(request: Request) Promise<Response> {{
{defer_secret_js}    const path = request.pathname == "" ? "/" : request.pathname
    const accept = request.headers.accept
    const fragment = accept == "{FRAGMENT_ACCEPT}" || accept == "{FRAGMENT_ACCEPT_LEGACY}"
    const staticBuild = accept == "{STATIC_ACCEPT}"
{branches}    return await respond({not_found_tree}, 404, fragment, staticBuild, {not_found_head})
}}
export {{ App }}
"#
    ))
}
fn split_document(index_html: &str, scripts: &str) -> (String, String, String) {
    let no_scripts = index_html.replace(DEKA_SCRIPTS_HOLE, scripts);
    let (before_app, after_app) = match no_scripts.split_once(DEKA_APP_HOLE) {
        Some((a, b)) => (a, b),
        None => (no_scripts.as_str(), ""),
    };
    if let Some((before_head, after_head)) = before_app.split_once(DEKA_HEAD_HOLE) {
        (
            before_head.to_string(),
            after_head.to_string(),
            after_app.to_string(),
        )
    } else {
        (before_app.to_string(), String::new(), after_app.to_string())
    }
}
fn with_css_links(plan: &CssPlan, route: &str, head_js: String) -> Result<String, String> {
    let links = css_links_for_route(plan, route);
    if links.is_empty() {
        return Ok(head_js);
    }
    Ok(format!("{} + {}", json_str(&links)?, head_js))
}
pub(super) fn path_condition(route: &str) -> Result<String, String> {
    assert_dynamic_route_supported(route)?;
    if route == "/" {
        return Ok("path == \"/\" || path == \"\"".to_string());
    }
    if !route.contains('[') {
        return Ok(format!("path == {}", json_str(route)?));
    }
    let prefix = static_prefix(route);
    Ok(format!("one_segment_after(path, {})", json_str(&prefix)?))
}
pub(super) fn wrap_layouts(entries: &[FrameworkEntry], route: &str, page_alias: &str) -> String {
    let mut expr = page_call(route, page_alias);
    for seg in ancestor_routes(route) {
        if let Some(loading) = loading_at(entries, &seg) {
            expr = suspense_wrap(&alias("Loading", &loading.route), &expr);
        }
        if let Some(layout) = layout_at(entries, &seg) {
            let name = alias("Layout", &layout.route);
            expr = format!("<{name}>{expr}</{name}>");
        }
    }
    expr
}
fn layout_at<'a>(entries: &'a [FrameworkEntry], route: &str) -> Option<&'a FrameworkEntry> {
    entries
        .iter()
        .find(|entry| entry.kind == FrameworkEntryKind::Layout && entry.route == route)
}

fn loading_at<'a>(entries: &'a [FrameworkEntry], route: &str) -> Option<&'a FrameworkEntry> {
    entries
        .iter()
        .find(|entry| entry.kind == FrameworkEntryKind::Loading && entry.route == route)
}

fn suspense_wrap(loading_alias: &str, children: &str) -> String {
    format!("<Suspense fallback={{<{loading_alias} />}}>{children}</Suspense>")
}

pub(super) fn page_call(route: &str, page_alias: &str) -> String {
    let params = dynamic_param_names(route);
    if params.is_empty() {
        format!("<{page_alias} />")
    } else {
        let attrs = params
            .iter()
            .map(|name| format!("{name}={{last_segment(path)}}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("<{page_alias} {attrs} />")
    }
}
fn layout_head_aliases(entries: &[FrameworkEntry], route: &str) -> Vec<String> {
    layout_chain(entries, route)
        .into_iter()
        .filter(|layout| exports_head(Path::new(&layout.file)))
        .map(|layout| format!("{}()", alias("headL", &layout.route)))
        .collect()
}

fn head_concat(entries: &[FrameworkEntry], page: &FrameworkEntry, not_found: bool) -> String {
    let mut parts = layout_head_aliases(entries, &page.route);
    if !not_found && exports_head(Path::new(&page.file)) {
        parts.push(format!("{}()", alias("head", &page.route)));
    }
    if parts.is_empty() {
        "\"\"".to_string()
    } else {
        head_expr(&parts, "")
    }
}

fn head_expr(calls: &[String], extra: &str) -> String {
    let mut bits: Vec<String> = calls
        .iter()
        .map(|call| format!("head_html({call})"))
        .collect();
    if !extra.is_empty() && extra != "\"\"" {
        bits.push(format!("head_html({extra})"));
    }
    if bits.is_empty() {
        "\"\"".to_string()
    } else {
        bits.join(" + ")
    }
}
