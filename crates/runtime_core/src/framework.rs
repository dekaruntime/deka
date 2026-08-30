use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameworkEntryKind {
    Page,
    Layout,
    Api,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkEntry {
    pub kind: FrameworkEntryKind,
    pub route: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FrameworkManifest {
    pub root: String,
    pub entries: Vec<FrameworkEntry>,
    pub not_found: Option<FrameworkEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch<'a> {
    pub page: Option<&'a FrameworkEntry>,
    pub layouts: Vec<&'a FrameworkEntry>,
    pub status: u16,
    pub params: BTreeMap<String, String>,
}

pub fn route_from_relative_path(kind: FrameworkEntryKind, relative_path: &str) -> Option<String> {
    let normalized = relative_path.replace('\\', "/");
    let trimmed = normalized.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let suffixes: &[&str] = match kind {
        FrameworkEntryKind::Page => &["/page.dsx", "/page.ds", "/page.phpx"],
        FrameworkEntryKind::Layout => &["/layout.dsx", "/layout.ds", "/layout.phpx"],
        FrameworkEntryKind::Api => &[".ds", ".dsx", ".phpx"],
    };

    let route_source = suffixes.iter().find_map(|suffix| {
        let bare = suffix.trim_start_matches('/');
        if trimmed == bare {
            Some("")
        } else {
            trimmed.strip_suffix(suffix)
        }
    })?;

    if route_source.is_empty() {
        return Some("/".to_string());
    }

    Some(format!("/{}", route_source.trim_matches('/')))
}

pub const DEKA_HEAD_HOLE: &str = "<!--deka-head-->";
pub const DEKA_APP_HOLE: &str = "<!--deka-app-->";
pub const DEKA_SCRIPTS_HOLE: &str = "<!--deka-scripts-->";
pub const FRAGMENT_ACCEPT: &str = "text/x-deka-fragment";
pub const FRAGMENT_ACCEPT_LEGACY: &str = "text/x-phpx-fragment";

/// Fill the three document holes. Missing holes are left unchanged.
pub fn fill_document(index_html: &str, head: &str, app: &str, scripts: &str) -> String {
    let mut out = index_html.to_string();
    if out.contains(DEKA_HEAD_HOLE) {
        out = out.replacen(DEKA_HEAD_HOLE, head, 1);
    }
    if out.contains(DEKA_APP_HOLE) {
        out = out.replacen(DEKA_APP_HOLE, app, 1);
    }
    if out.contains(DEKA_SCRIPTS_HOLE) {
        out = out.replacen(DEKA_SCRIPTS_HOLE, scripts, 1);
    }
    out
}

/// True when the project uses the RFD 24 document + `app/page.dsx` shape.
pub fn is_app_router_project(project_root: &Path) -> bool {
    project_root.join("index.html").is_file()
        && (project_root.join("app/page.dsx").is_file() || project_root.join("app/page.ds").is_file())
}

pub fn normalize_request_path(raw: &str) -> String {
    let without_query = raw.split('?').next().unwrap_or(raw);
    let trimmed = without_query.trim();
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let mut path = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    if path.len() > 1 {
        while path.ends_with('/') {
            path.pop();
        }
    }
    if path.is_empty() {
        "/".to_string()
    } else {
        path
    }
}

/// Layout files from the root down to `route`, inclusive. Missing intermediate
/// layouts are skipped; the root layout is included when present.
pub fn layout_chain<'a>(entries: &'a [FrameworkEntry], route: &str) -> Vec<&'a FrameworkEntry> {
    let mut layouts: Vec<&FrameworkEntry> = entries
        .iter()
        .filter(|entry| entry.kind == FrameworkEntryKind::Layout)
        .filter(|entry| layout_applies(entry.route.as_str(), route))
        .collect();
    layouts.sort_by_key(|entry| entry.route.len());
    layouts
}

fn layout_applies(layout_route: &str, page_route: &str) -> bool {
    if layout_route == "/" {
        return true;
    }
    page_route == layout_route
        || page_route.starts_with(&format!("{}/", layout_route.trim_end_matches('/')))
}

pub fn scan_app_dir(app_dir: &Path) -> FrameworkManifest {
    let mut manifest = FrameworkManifest {
        root: app_dir.to_string_lossy().into_owned(),
        entries: Vec::new(),
        not_found: None,
    };
    if !app_dir.is_dir() {
        return manifest;
    }
    visit_app_dir(app_dir, app_dir, &mut manifest);
    manifest.entries.sort_by(|a, b| a.route.cmp(&b.route).then(a.file.cmp(&b.file)));
    manifest
}

fn visit_app_dir(app_root: &Path, dir: &Path, manifest: &mut FrameworkManifest) {
    let Ok(reader) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in reader.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            visit_app_dir(app_root, &path, manifest);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(app_root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if is_not_found_file(&name) {
            if rel.contains('/') {
                continue;
            }
            manifest.not_found = Some(FrameworkEntry {
                kind: FrameworkEntryKind::Page,
                route: "/".to_string(),
                file: path.to_string_lossy().into_owned(),
            });
            continue;
        }
        if let Some(route) = route_from_relative_path(FrameworkEntryKind::Page, &rel) {
            manifest.entries.push(FrameworkEntry {
                kind: FrameworkEntryKind::Page,
                route,
                file: path.to_string_lossy().into_owned(),
            });
            continue;
        }
        if let Some(route) = route_from_relative_path(FrameworkEntryKind::Layout, &rel) {
            manifest.entries.push(FrameworkEntry {
                kind: FrameworkEntryKind::Layout,
                route,
                file: path.to_string_lossy().into_owned(),
            });
        }
    }
}

fn is_not_found_file(name: &str) -> bool {
    matches!(
        name,
        "not-found.dsx" | "not-found.ds" | "not-found.phpx"
    )
}

pub fn match_path<'a>(manifest: &'a FrameworkManifest, raw_path: &str) -> RouteMatch<'a> {
    let path = normalize_request_path(raw_path);
    let mut pages: Vec<&FrameworkEntry> = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == FrameworkEntryKind::Page)
        .collect();
    pages.sort_by_key(|entry| dynamic_rank(&entry.route));

    for page in pages {
        if let Some(params) = route_pattern_matches(&page.route, &path) {
            return RouteMatch {
                layouts: layout_chain(&manifest.entries, &page.route),
                page: Some(page),
                status: 200,
                params,
            };
        }
    }

    RouteMatch {
        layouts: layout_chain(&manifest.entries, "/"),
        page: manifest.not_found.as_ref(),
        status: 404,
        params: BTreeMap::new(),
    }
}

fn dynamic_rank(route: &str) -> (u8, usize) {
    let dynamic = route.contains('[') as u8;
    (dynamic, usize::MAX - route.len())
}

pub fn route_pattern_matches(pattern: &str, path: &str) -> Option<BTreeMap<String, String>> {
    let pat: Vec<&str> = pattern.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    let seg: Vec<&str> = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    if pattern == "/" {
        return if path == "/" {
            Some(BTreeMap::new())
        } else {
            None
        };
    }
    if pat.len() != seg.len() {
        return None;
    }
    let mut params = BTreeMap::new();
    for (p, s) in pat.iter().zip(seg.iter()) {
        if let Some(name) = p.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
            if name.starts_with("...") {
                return None;
            }
            params.insert(name.to_string(), (*s).to_string());
            continue;
        }
        if p != s {
            return None;
        }
    }
    Some(params)
}

pub fn static_page_routes(manifest: &FrameworkManifest) -> Vec<String> {
    manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == FrameworkEntryKind::Page && !entry.route.contains('['))
        .map(|entry| entry.route.clone())
        .collect()
}

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
    let cache_dir = project_root.join(".cache").join("dekascript");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("serve-entry.dsx");
    let source = generate_serve_entry(&entry, &manifest, &index_html)?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}

fn generate_serve_entry(
    entry: &Path,
    manifest: &FrameworkManifest,
    index_html: &str,
) -> Result<String, String> {
    let (doc_head, doc_mid, doc_tail) = split_document(index_html);
    let doc_head = json_str(&doc_head)?;
    let doc_mid = json_str(&doc_mid)?;
    let doc_tail = json_str(&doc_tail)?;

    let mut imports = String::new();
    let mut imported: Vec<String> = Vec::new();
    let mut import_alias = |path: &str, name: &str, alias: &str| {
        if imported.iter().any(|k| k == alias) {
            return;
        }
        imported.push(alias.to_string());
        let rel = pathdiff_dsx(entry, Path::new(path));
        imports.push_str(&format!("import {{ {name} as {alias} }} from \"{rel}\"\n"));
    };

    let mut pages: Vec<&FrameworkEntry> = manifest
        .entries
        .iter()
        .filter(|e| e.kind == FrameworkEntryKind::Page)
        .collect();
    pages.sort_by_key(|e| dynamic_rank(&e.route));

    for page in &pages {
        import_alias(&page.file, "Page", &alias("Page", &page.route));
        if exports_head(Path::new(&page.file)) {
            import_alias(&page.file, "head", &alias("head", &page.route));
        }
    }
    for layout in manifest
        .entries
        .iter()
        .filter(|e| e.kind == FrameworkEntryKind::Layout)
    {
        import_alias(&layout.file, "Layout", &alias("Layout", &layout.route));
        if exports_head(Path::new(&layout.file)) {
            import_alias(&layout.file, "head", &alias("headL", &layout.route));
        }
    }
    if let Some(not_found) = &manifest.not_found {
        import_alias(&not_found.file, "Page", "Page_not_found");
        if exports_head(Path::new(&not_found.file)) {
            import_alias(&not_found.file, "head", "head_not_found");
        }
    }

    let mut branches = String::new();
    for page in &pages {
        let cond = path_condition(&page.route);
        let tree = wrap_layouts(&manifest.entries, &page.route, &alias("Page", &page.route));
        let head = head_concat(&manifest.entries, page, false);
        branches.push_str(&format!(
            "    if ({cond}) {{\n        return respond({tree}, 200, fragment, {head})\n    }}\n"
        ));
    }
    let not_found_tree = if manifest.not_found.is_some() {
        wrap_layouts(&manifest.entries, "/", "Page_not_found")
    } else {
        "FallbackNotFound()".to_string()
    };
    let not_found_head = if manifest.not_found.as_ref().is_some_and(|e| exports_head(Path::new(&e.file))) {
        head_expr(&layout_head_aliases(&manifest.entries, "/"), "head_not_found()")
    } else {
        head_expr(&layout_head_aliases(&manifest.entries, "/"), "\"\"")
    };

    let fallback_fn = if manifest.not_found.is_some() {
        String::new()
    } else {
        "fn FallbackNotFound() {\n    return <section><h1>Not found</h1></section>;\n}\n\n".to_string()
    };

    Ok(format!(
        r#"{imports}
interface RequestHeaders {{ accept: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders }}
interface Response {{ status: number, body: string }}

{fallback_fn}fn starts_with(s: string, prefix: string): boolean {{
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

fn one_segment_after(s: string, prefix: string): boolean {{
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

fn last_segment(s: string): string {{
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

fn head_html(node: Component): string {{
    const result = unsafe {{ deka.ui.renderToString(node) }}
    return match (result) {{
        Ok(rendered) => rendered.html,
        Err(_) => "",
    }}
}}

fn respond(tree: Component, status: number, fragment: boolean, headHtml: string): Response {{
    const result = unsafe {{ deka.ui.renderToString(tree) }}
    const appHtml = match (result) {{
        Ok(rendered) => rendered.html,
        Err(err) => err.message,
    }}
    if (fragment) {{
        const payload = unsafe {{ JSON.stringify({{ html: appHtml, head: headHtml }}) }}
        return match (payload) {{
            Ok(json) => {{ status: status, body: json }},
            Err(err) => {{ status: 500, body: err.message }},
        }}
    }}
    return {{ status: status, body: {doc_head} + headHtml + {doc_mid} + appHtml + {doc_tail} }}
}}

export fn App(request: Request): Response {{
    const path = request.pathname == "" ? "/" : request.pathname
    const accept = request.headers.accept
    const fragment = accept == "{FRAGMENT_ACCEPT}" || accept == "{FRAGMENT_ACCEPT_LEGACY}"
{branches}    return respond({not_found_tree}, 404, fragment, {not_found_head})
}}
"#
    ))
}

fn split_document(index_html: &str) -> (String, String, String) {
    let no_scripts = index_html.replace(DEKA_SCRIPTS_HOLE, "");
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

fn json_str(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| format!("failed to encode string: {err}"))
}

fn exports_head(path: &Path) -> bool {
    let Ok(src) = std::fs::read_to_string(path) else {
        return false;
    };
    src.contains("export fn head") || src.contains("export function head")
}

fn alias(prefix: &str, route: &str) -> String {
    let mut out = prefix.to_string();
    if route == "/" {
        out.push_str("_root");
        return out;
    }
    out.push('_');
    for ch in route.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn path_condition(route: &str) -> String {
    if route == "/" {
        return "path == \"/\" || path == \"\"".to_string();
    }
    if !route.contains('[') {
        return format!("path == \"{}\"", route);
    }
    let prefix = static_prefix(route);
    format!("one_segment_after(path, \"{prefix}\")")
}

fn static_prefix(route: &str) -> String {
    let mut parts = Vec::new();
    for part in route.trim_matches('/').split('/') {
        if part.starts_with('[') {
            break;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", parts.join("/"))
    }
}

fn wrap_layouts(entries: &[FrameworkEntry], route: &str, page_alias: &str) -> String {
    let mut expr = page_call(route, page_alias);
    for layout in layout_chain(entries, route).into_iter().rev() {
        let name = alias("Layout", &layout.route);
        expr = format!("{name}({{ children: {expr} }})");
    }
    expr
}

fn page_call(route: &str, page_alias: &str) -> String {
    let params = dynamic_param_names(route);
    if params.is_empty() {
        format!("{page_alias}()")
    } else {
        let fields = params
            .iter()
            .map(|name| format!("{name}: last_segment(path)"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{page_alias}({{ {fields} }})")
    }
}

fn dynamic_param_names(route: &str) -> Vec<String> {
    route
        .trim_matches('/')
        .split('/')
        .filter_map(|part| {
            let name = part.strip_prefix('[')?.strip_suffix(']')?;
            if name.is_empty() || name.starts_with("...") {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
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

fn pathdiff_dsx(from_file: &Path, to_file: &Path) -> String {
    let from_dir = from_file.parent().unwrap_or(Path::new("."));
    let mut rel = pathdiff(from_dir, to_file);
    if !rel.starts_with('.') {
        rel = format!("./{rel}");
    }
    rel.replace('\\', "/")
}

fn pathdiff(from_dir: &Path, to_file: &Path) -> String {
    let from = from_dir.components().collect::<Vec<_>>();
    let to = to_file.components().collect::<Vec<_>>();
    let mut i = 0;
    while i < from.len() && i < to.len() && from[i] == to[i] {
        i += 1;
    }
    let mut parts = Vec::new();
    for _ in i..from.len() {
        parts.push("..");
    }
    for component in to.iter().skip(i) {
        parts.push(component.as_os_str().to_str().unwrap_or(""));
    }
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_root_page_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "page.phpx"),
            Some("/".to_string())
        );
    }

    #[test]
    fn derives_nested_page_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "users/page.phpx"),
            Some("/users".to_string())
        );
    }

    #[test]
    fn derives_layout_scope_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Layout, "dashboard/layout.phpx"),
            Some("/dashboard".to_string())
        );
    }

    #[test]
    fn derives_api_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Api, "api/packages.phpx"),
            Some("/api/packages".to_string())
        );
    }

    #[test]
    fn preserves_dynamic_segments() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "blog/[slug]/page.phpx"),
            Some("/blog/[slug]".to_string())
        );
    }

    #[test]
    fn derives_dsx_page_and_layout_routes() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "page.dsx"),
            Some("/".to_string())
        );
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "users/page.dsx"),
            Some("/users".to_string())
        );
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Layout, "layout.dsx"),
            Some("/".to_string())
        );
    }

    #[test]
    fn fill_document_replaces_the_three_holes() {
        let index = "<head><!--deka-head--></head><body><!--deka-app--><!--deka-scripts--></body>";
        let filled = fill_document(index, "<title>Hi</title>", "<p>app</p>", "");
        assert_eq!(
            filled,
            "<head><title>Hi</title></head><body><p>app</p></body>"
        );
    }

    #[test]
    fn layout_chain_is_root_then_nested() {
        let entries = vec![
            FrameworkEntry {
                kind: FrameworkEntryKind::Layout,
                route: "/".to_string(),
                file: "app/layout.dsx".to_string(),
            },
            FrameworkEntry {
                kind: FrameworkEntryKind::Layout,
                route: "/blog".to_string(),
                file: "app/blog/layout.dsx".to_string(),
            },
            FrameworkEntry {
                kind: FrameworkEntryKind::Page,
                route: "/blog".to_string(),
                file: "app/blog/page.dsx".to_string(),
            },
        ];
        let chain = layout_chain(&entries, "/blog");
        assert_eq!(
            chain.iter().map(|e| e.route.as_str()).collect::<Vec<_>>(),
            vec!["/", "/blog"]
        );
    }

    #[test]
    fn match_path_uses_not_found_for_unknown_routes() {
        let manifest = FrameworkManifest {
            root: "app".into(),
            entries: vec![
                FrameworkEntry {
                    kind: FrameworkEntryKind::Layout,
                    route: "/".into(),
                    file: "app/layout.dsx".into(),
                },
                FrameworkEntry {
                    kind: FrameworkEntryKind::Page,
                    route: "/".into(),
                    file: "app/page.dsx".into(),
                },
            ],
            not_found: Some(FrameworkEntry {
                kind: FrameworkEntryKind::Page,
                route: "/".into(),
                file: "app/not-found.dsx".into(),
            }),
        };
        let hit = match_path(&manifest, "/");
        assert_eq!(hit.status, 200);
        assert_eq!(hit.page.unwrap().file, "app/page.dsx");
        let miss = match_path(&manifest, "/missing");
        assert_eq!(miss.status, 404);
        assert_eq!(miss.page.unwrap().file, "app/not-found.dsx");
    }

    #[test]
    fn match_path_binds_dynamic_segments() {
        let manifest = FrameworkManifest {
            root: "app".into(),
            entries: vec![FrameworkEntry {
                kind: FrameworkEntryKind::Page,
                route: "/blog/[slug]".into(),
                file: "app/blog/[slug]/page.dsx".into(),
            }],
            not_found: None,
        };
        let hit = match_path(&manifest, "/blog/hello");
        assert_eq!(hit.status, 200);
        assert_eq!(hit.params.get("slug").map(String::as_str), Some("hello"));
        let miss = match_path(&manifest, "/blog/hello/extra");
        assert_eq!(miss.status, 404);
    }

    #[test]
    fn scan_app_dir_finds_nested_page_and_not_found() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_scan_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("blog")).unwrap();
        std::fs::write(tmp.join("page.dsx"), "export fn Page() { return <p>home</p>; }\n").unwrap();
        std::fs::write(tmp.join("layout.dsx"), "export fn Layout(props: LayoutProps) { return <main>{props.children}</main>; }\n").unwrap();
        std::fs::write(tmp.join("not-found.dsx"), "export fn Page() { return <p>404</p>; }\n").unwrap();
        std::fs::write(tmp.join("blog/page.dsx"), "export fn Page() { return <p>blog</p>; }\n").unwrap();
        let manifest = scan_app_dir(&tmp);
        assert!(manifest.not_found.is_some());
        assert!(manifest.entries.iter().any(|e| e.route == "/" && e.kind == FrameworkEntryKind::Page));
        assert!(manifest.entries.iter().any(|e| e.route == "/blog" && e.kind == FrameworkEntryKind::Page));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn page_call_passes_slug_from_last_segment() {
        assert_eq!(page_call("/", "Page_root"), "Page_root()");
        assert_eq!(
            page_call("/blog/[slug]", "Page_blog__slug_"),
            "Page_blog__slug_({ slug: last_segment(path) })"
        );
        assert_eq!(dynamic_param_names("/blog/[slug]"), vec!["slug".to_string()]);
        assert!(dynamic_param_names("/about").is_empty());
    }

    #[test]
    fn generated_serve_entry_merges_head_and_passes_slug() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_gen_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("app/blog/[slug]")).unwrap();
        std::fs::write(
            tmp.join("index.html"),
            "<!doctype html><html><head><!--deka-head--></head><body><div id=\"app\"><!--deka-app--></div><!--deka-scripts--></body></html>\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("app/layout.dsx"),
            "interface LayoutProps { children: Component }\nexport fn Layout(props: LayoutProps) {\n    return <main>{props.children}</main>;\n}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn head() {\n    return <title>Head Merge</title>;\n}\nexport fn Page() {\n    return <section><h1>Home</h1></section>;\n}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("app/blog/[slug]/page.dsx"),
            "interface PageProps { slug: string }\nexport fn Page(props: PageProps) {\n    return <article>{props.slug}</article>;\n}\n",
        )
        .unwrap();
        let entry = write_app_router_entry(&tmp).expect("generate serve-entry");
        let source = std::fs::read_to_string(&entry).expect("read serve-entry");
        assert!(
            source.contains("head_html(head_root())"),
            "generated entry should merge page head(): {source}"
        );
        assert!(
            source.contains("last_segment"),
            "generated entry should define last_segment: {source}"
        );
        assert!(
            source.contains("slug: last_segment(path)"),
            "generated [slug] page call should pass params: {source}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
