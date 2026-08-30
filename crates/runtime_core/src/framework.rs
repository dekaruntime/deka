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
pub fn is_app_router_project(project_root: &std::path::Path) -> bool {
    project_root.join("index.html").is_file()
        && (project_root.join("app/page.dsx").is_file() || project_root.join("app/page.ds").is_file())
}

/// Layout files from the root down to `route`, inclusive. Root layout is required
/// for the `/` page; missing intermediate layouts are skipped.
pub fn layout_chain<'a>(
    entries: &'a [FrameworkEntry],
    route: &str,
) -> Vec<&'a FrameworkEntry> {
    let mut layouts: Vec<&FrameworkEntry> = entries
        .iter()
        .filter(|entry| entry.kind == FrameworkEntryKind::Layout)
        .filter(|entry| route == entry.route || route.starts_with(&format!("{}/", entry.route.trim_end_matches('/'))) || entry.route == "/")
        .collect();
    layouts.sort_by_key(|entry| entry.route.len());
    layouts
}

/// Write a generated App() entry that renders `app/page.dsx` through the root
/// layout into the project `index.html` holes. Used by `deka serve` when the
/// project has no `serve.entry`.
pub fn write_app_router_entry(project_root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let page = ["app/page.dsx", "app/page.ds"]
        .iter()
        .map(|rel| project_root.join(rel))
        .find(|path| path.is_file())
        .ok_or_else(|| "app router requires app/page.dsx".to_string())?;
    let layout = ["app/layout.dsx", "app/layout.ds"]
        .iter()
        .map(|rel| project_root.join(rel))
        .find(|path| path.is_file())
        .ok_or_else(|| "app router requires app/layout.dsx (root layout)".to_string())?;
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
    let page_import = pathdiff_dsx(&entry, &page);
    let layout_import = pathdiff_dsx(&entry, &layout);
    let stripped = index_html.replace(DEKA_SCRIPTS_HOLE, "");
    let (head, tail) = match stripped.split_once(DEKA_APP_HOLE) {
        Some((head, tail)) => (head, tail),
        None => (stripped.as_str(), ""),
    };
    let head_json = serde_json::to_string(head)
        .map_err(|err| format!("failed to encode index.html: {err}"))?;
    let tail_json = serde_json::to_string(tail)
        .map_err(|err| format!("failed to encode index.html: {err}"))?;
    let source = format!(
        r#"import {{ Page }} from "{page_import}"
import {{ Layout }} from "{layout_import}"

interface Request {{ url: string }}
interface Response {{ status: number, body: string }}

export fn App(request: Request): Response {{
    const inner = Page()
    const tree = Layout({{ children: inner }})
    const result = unsafe {{ deka.ui.renderToString(tree) }}
    return match (result) {{
        Ok(rendered) => {{ status: 200, body: {head_json} + rendered.html + {tail_json} }},
        Err(err) => {{ status: 500, body: err.message }},
    }}
}}
"#
    );
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}

fn pathdiff_dsx(from_file: &std::path::Path, to_file: &std::path::Path) -> String {
    let from_dir = from_file.parent().unwrap_or(std::path::Path::new("."));
    let mut rel = pathdiff(from_dir, to_file);
    if !rel.starts_with('.') {
        rel = format!("./{rel}");
    }
    rel.replace('\\', "/")
}

fn pathdiff(from_dir: &std::path::Path, to_file: &std::path::Path) -> String {
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
    use super::{
        FrameworkEntry, FrameworkEntryKind, fill_document, layout_chain, route_from_relative_path,
    };

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
}
