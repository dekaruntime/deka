use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameworkEntryKind {
    Page,
    Layout,
    Api,
    Loading,
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
        FrameworkEntryKind::Page => &["/page.dsx", "/page.ds"],
        FrameworkEntryKind::Layout => &["/layout.dsx", "/layout.ds"],
        FrameworkEntryKind::Loading => &["/loading.dsx", "/loading.ds"],
        FrameworkEntryKind::Api => &[".ds", ".dsx"],
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
pub const STATIC_ACCEPT: &str = "text/x-deka-static";
pub const MIDDLEWARE_FILE: &str = "middleware.ds";
/// Middleware App() uses status 0 to mean "continue to api/ or app/".
pub const MIDDLEWARE_NEXT_STATUS: u16 = 0;

/// Fill the three document holes. Missing holes are left unchanged.
pub fn fill_document(index_html: &str, head: &str, app: &str, scripts: &str) -> String {
    // Replace holes in template order, each once. App HTML is escaped by
    // renderToString so it cannot contain a raw `<!--deka-scripts-->` that
    // would steal the later pass.
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
    let mut path = format!("/{}", trimmed.trim_start_matches('/'));
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
            continue;
        }
        if let Some(route) = route_from_relative_path(FrameworkEntryKind::Loading, &rel) {
            manifest.entries.push(FrameworkEntry {
                kind: FrameworkEntryKind::Loading,
                route,
                file: path.to_string_lossy().into_owned(),
            });
        }
    }
}

fn is_not_found_file(name: &str) -> bool {
    matches!(name, "not-found.dsx" | "not-found.ds")
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

const HTTP_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

pub fn scan_api_dir(api_dir: &Path) -> Vec<FrameworkEntry> {
    let mut entries = Vec::new();
    if !api_dir.is_dir() {
        return entries;
    }
    visit_api_dir(api_dir, api_dir, &mut entries);
    entries.sort_by(|a, b| a.route.cmp(&b.route));
    entries
}

fn visit_api_dir(api_root: &Path, dir: &Path, entries: &mut Vec<FrameworkEntry>) {
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
            visit_api_dir(api_root, &path, entries);
            continue;
        }
        if !matches!(name.as_ref(), "route.ds" | "route.dsx") {
            continue;
        }
        let rel = path
            .strip_prefix(api_root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let parent = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let route = if parent.is_empty() {
            "/api".to_string()
        } else {
            format!("/api/{parent}")
        };
        entries.push(FrameworkEntry {
            kind: FrameworkEntryKind::Api,
            route,
            file: path.to_string_lossy().into_owned(),
        });
    }
}

pub fn exported_http_methods(path: &Path) -> Vec<String> {
    let Ok(src) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let stripped = strip_ds_comments(&src);
    HTTP_METHODS
        .iter()
        .copied()
        .filter(|method| exports_fn_named(&stripped, method))
        .map(|m| m.to_string())
        .collect()
}

fn exports_fn_named(src: &str, name: &str) -> bool {
    export_fn_needles(name)
        .into_iter()
        .any(|needle| contains_export_fn(src, &needle))
        || export_list_contains(src, name)
}

fn export_fn_needles(name: &str) -> Vec<String> {
    vec![
        format!("export fn {name}"),
        format!("export async fn {name}"),
        format!("export function {name}"),
        format!("export async function {name}"),
    ]
}

fn contains_export_fn(src: &str, needle: &str) -> bool {
    let mut rest = src;
    while let Some(at) = rest.find(needle) {
        let after = &rest[at + needle.len()..];
        let boundary = match after.chars().next() {
            None => true,
            Some(c) => !c.is_ascii_alphanumeric() && c != '_',
        };
        if boundary {
            return true;
        }
        rest = &rest[at + 1..];
    }
    false
}

fn export_list_contains(src: &str, name: &str) -> bool {
    let mut rest = src;
    while let Some(at) = rest.find("export {") {
        let after = &rest[at + "export {".len()..];
        let Some(end) = after.find('}') else {
            break;
        };
        for part in after[..end].split(',') {
            let ident = part
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('|');
            if ident == name {
                return true;
            }
        }
        rest = &after[end.saturating_add(1)..];
    }
    false
}

fn strip_ds_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = i.saturating_add(2).min(bytes.len());
            continue;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            out.push(quote as char);
            i += 1;
            while i < bytes.len() {
                out.push(bytes[i] as char);
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    if i < bytes.len() {
                        out.push(bytes[i] as char);
                    }
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIsland {
    pub component: String,
    pub directive: String,
    pub file: String,
    pub props: Vec<String>,
}

pub fn scan_client_islands(app_dir: &Path) -> Vec<ClientIsland> {
    let mut out = Vec::new();
    if !app_dir.is_dir() {
        return out;
    }
    let mut stack = vec![app_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(reader) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in reader.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if ext != "dsx" && ext != "ds" {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.extend(islands_in_source(&src, path.to_string_lossy().as_ref()));
        }
    }
    out
}

fn islands_in_source(src: &str, file: &str) -> Vec<ClientIsland> {
    let stripped = strip_ds_comments(src);
    islands_in_source_raw(&stripped, file)
}

fn islands_in_source_raw(src: &str, file: &str) -> Vec<ClientIsland> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let Some(rel) = ["client:load", "client:idle", "client:visible"]
            .iter()
            .filter_map(|needle| rest.find(needle).map(|at| (at, *needle)))
            .min_by_key(|(at, _)| *at)
        else {
            break;
        };
        let at = i + rel.0;
        let directive = rel.1.rsplit(':').next().unwrap_or("load").to_string();
        let prefix = &src[..at];
        let tag_start = prefix.rfind('<').unwrap_or(0);
        let tag_end = src[tag_start..]
            .find('>')
            .map(|rel| tag_start + rel + 1)
            .unwrap_or(at + rel.1.len());
        let tag_src = &src[tag_start..tag_end];
        let component = tag_src
            .trim_start_matches('<')
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("")
            .to_string();
        let is_component = component
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
        if is_component {
            let props = island_prop_names(tag_src);
            out.push(ClientIsland {
                component,
                directive,
                file: file.to_string(),
                props,
            });
        }
        i = at + rel.1.len();
    }
    out
}

fn island_prop_names(tag_src: &str) -> Vec<String> {
    let mut names = Vec::new();
    for raw in tag_src.split_whitespace() {
        let name = raw.split('=').next().unwrap_or("").trim();
        if name.is_empty() || name.starts_with('<') || name.starts_with("client:") || name.starts_with("server:") {
            continue;
        }
        if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            names.push(name.to_string());
        }
    }
    names
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteStyle {
    pub route: String,
    pub classes: BTreeSet<String>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CssPlan {
    pub common: bool,
    pub routes: BTreeMap<String, bool>,
    pub common_classes: BTreeSet<String>,
    pub common_files: BTreeSet<String>,
}

pub fn route_css_slug(route: &str) -> String {
    if route == "/" {
        return "root".to_string();
    }
    let mut out = String::new();
    for ch in route.trim_matches('/').chars() {
        match ch {
            '/' => out.push_str("-s-"),
            '_' => out.push_str("-u-"),
            '-' => out.push_str("-h-"),
            '[' => out.push_str("-l-"),
            ']' => out.push_str("-r-"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ => out.push_str("-x-"),
        }
    }
    if out.is_empty() {
        "root".to_string()
    } else {
        out
    }
}

pub fn css_links_for_route(plan: &CssPlan, route: &str) -> String {
    let mut tags = String::new();
    if plan.common {
        tags.push_str("<link rel=\"stylesheet\" href=\"/assets/css/common.css\">");
    }
    if plan.routes.get(route).copied().unwrap_or(false) {
        tags.push_str(&format!(
            "<link rel=\"stylesheet\" href=\"/assets/css/route-{}.css\">",
            route_css_slug(route)
        ));
    }
    tags
}

pub fn collect_route_styles(manifest: &FrameworkManifest) -> Vec<RouteStyle> {
    let mut pages: Vec<&FrameworkEntry> = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == FrameworkEntryKind::Page)
        .collect();
    pages.sort_by_key(|entry| entry.route.clone());
    let mut out = Vec::new();
    for page in pages {
        let mut files = Vec::new();
        files.extend(layout_chain(&manifest.entries, &page.route).into_iter().map(|e| e.file.clone()));
        files.push(page.file.clone());
        let scanned = scan_style_graph(&files);
        out.push(RouteStyle {
            route: page.route.clone(),
            classes: scanned.0,
            files: scanned.1,
        });
    }
    if let Some(not_found) = &manifest.not_found {
        let mut files = Vec::new();
        files.extend(layout_chain(&manifest.entries, "/").into_iter().map(|e| e.file.clone()));
        files.push(not_found.file.clone());
        let scanned = scan_style_graph(&files);
        out.push(RouteStyle {
            route: "__not_found".to_string(),
            classes: scanned.0,
            files: scanned.1,
        });
    }
    out
}

pub fn css_plan_from_styles(styles: &[RouteStyle]) -> CssPlan {
    let mut class_count: BTreeMap<String, usize> = BTreeMap::new();
    let mut file_count: BTreeMap<String, usize> = BTreeMap::new();
    for style in styles {
        for class in &style.classes {
            *class_count.entry(class.clone()).or_insert(0) += 1;
        }
        let mut seen = BTreeSet::new();
        for file in &style.files {
            if seen.insert(file.clone()) {
                *file_count.entry(file.clone()).or_insert(0) += 1;
            }
        }
    }
    let mut common_classes = BTreeSet::new();
    for (class, count) in &class_count {
        if *count >= 2 {
            common_classes.insert(class.clone());
        }
    }
    let mut common_files = BTreeSet::new();
    for (file, count) in &file_count {
        if *count >= 2 {
            common_files.insert(file.clone());
        }
    }
    let any_styles = styles
        .iter()
        .any(|style| !style.classes.is_empty() || !style.files.is_empty());
    let common = any_styles;
    let mut routes = BTreeMap::new();
    for style in styles {
        let unique_class = style.classes.iter().any(|c| !common_classes.contains(c));
        let unique_file = style.files.iter().any(|f| !common_files.contains(f));
        routes.insert(style.route.clone(), unique_class || unique_file);
    }
    CssPlan {
        common,
        routes,
        common_classes,
        common_files,
    }
}

fn scan_style_graph(entry_files: &[String]) -> (BTreeSet<String>, Vec<String>) {
    let mut classes = BTreeSet::new();
    let mut css_files = Vec::new();
    let mut seen_css = BTreeSet::new();
    let mut seen_ds = BTreeSet::new();
    let mut stack = entry_files.to_vec();
    while let Some(file) = stack.pop() {
        if !seen_ds.insert(file.clone()) {
            continue;
        }
        let path = Path::new(&file);
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        classes.extend(collect_class_literals(&src));
        for css in collect_import_paths(&src, |p| p.to_ascii_lowercase().ends_with(".css")) {
            if let Some(resolved) = resolve_relative(path, &css) {
                if Path::new(&resolved).is_file() && seen_css.insert(resolved.clone()) {
                    css_files.push(resolved);
                }
            }
        }
        for ds in collect_import_paths(&src, |p| {
            let lower = p.to_ascii_lowercase();
            lower.ends_with(".ds") || lower.ends_with(".dsx")
        }) {
            if let Some(resolved) = resolve_relative(path, &ds) {
                stack.push(resolved);
            }
        }
    }
    (classes, css_files)
}

pub fn collect_class_literals(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = src.as_bytes();
    let mut i = 0usize;
    while i + 6 < bytes.len() {
        if !bytes[i..].starts_with(b"class=") {
            i += 1;
            continue;
        }
        i += 6;
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if quote == b'"' || quote == b'\'' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            push_classes(&src[start..i.min(src.len())], &mut out);
        } else if quote == b'{' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                push_classes(&src[start..i.min(src.len())], &mut out);
            }
        }
        i += 1;
    }
    out
}

fn push_classes(chunk: &str, out: &mut BTreeSet<String>) {
    for token in chunk.split_whitespace() {
        if !token.is_empty() {
            out.insert(token.to_string());
        }
    }
}

fn collect_import_paths(src: &str, pred: impl Fn(&str) -> bool) -> Vec<String> {
    let stripped = strip_ds_comments(src);
    let mut out = Vec::new();
    let mut rest = stripped.as_str();
    while let Some(at) = rest.find("import ") {
        let after = rest[at + "import ".len()..].trim_start();
        if let Some(spec) = import_specifier(after) {
            if pred(spec) {
                out.push(spec.to_string());
            }
        }
        rest = &rest[at + 1..];
    }
    out
}

fn import_specifier(after_import: &str) -> Option<&str> {
    let trimmed = after_import.trim_start();
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        return quoted_prefix(trimmed);
    }
    let from = trimmed.find(" from ")?;
    quoted_prefix(trimmed[from + 6..].trim_start())
}

fn quoted_prefix(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let quote = bytes[0];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let rest = &s[1..];
    let end = rest.find(quote as char)?;
    Some(&rest[..end])
}

fn resolve_relative(from_file: &Path, spec: &str) -> Option<String> {
    if !(spec.starts_with("./") || spec.starts_with("../")) {
        return None;
    }
    let parent = from_file.parent()?;
    let resolved = parent.join(spec);
    Some(resolved.to_string_lossy().into_owned())
}

pub fn island_script_tags(islands: &[ClientIsland]) -> String {
    let mut seen = std::collections::BTreeSet::new();
    let mut tags = String::new();
    for directive in ["load", "idle", "visible"] {
        if islands.iter().any(|i| i.directive == directive) && seen.insert(directive) {
            tags.push_str(&format!(
                "<script type=\"module\" src=\"/assets/islands-{directive}.js\"></script>"
            ));
        }
    }
    tags
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredIsland {
    pub component: String,
    pub file: String,
    pub props: Vec<String>,
    pub cache: Option<String>,
    pub has_fallback: bool,
}

pub fn scan_server_defer(app_dir: &Path) -> Vec<DeferredIsland> {
    let mut out = Vec::new();
    if !app_dir.is_dir() {
        return out;
    }
    let mut stack = vec![app_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(reader) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in reader.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if ext != "dsx" && ext != "ds" {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.extend(defer_in_source(&src, path.to_string_lossy().as_ref()));
        }
    }
    out
}

fn defer_in_source(src: &str, file: &str) -> Vec<DeferredIsland> {
    let stripped = strip_ds_comments(src);
    defer_in_source_raw(&stripped, file)
}

fn defer_in_source_raw(src: &str, file: &str) -> Vec<DeferredIsland> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let Some(rel) = rest.find("server:defer") else {
            break;
        };
        let at = i + rel;
        let prefix = &src[..at];
        let tag_start = prefix.rfind('<').unwrap_or(0);
        let tag_end = src[tag_start..]
            .find('>')
            .map(|rel| tag_start + rel + 1)
            .unwrap_or(at + "server:defer".len());
        let tag_src = &src[tag_start..tag_end];
        let component = tag_src
            .trim_start_matches('<')
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("")
            .to_string();
        let is_component = component
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
        if is_component {
            let self_closing = tag_src.trim_end().ends_with("/>") || tag_src.contains("/>");
            let has_fallback = !self_closing && fallback_in_element(src, tag_end, &component);
            let cache = defer_cache_attr(tag_src);
            out.push(DeferredIsland {
                component,
                file: file.to_string(),
                props: island_prop_names(tag_src),
                cache,
                has_fallback,
            });
        }
        i = at + "server:defer".len();
    }
    out
}

fn fallback_in_element(src: &str, tag_end: usize, component: &str) -> bool {
    let rest = &src[tag_end.min(src.len())..];
    let close = format!("</{component}>");
    let window = rest.find(&close).map(|i| &rest[..i]).unwrap_or(rest);
    window.contains("slot=\"fallback\"")
        || window.contains("slot='fallback'")
        || window.contains("slot={\"fallback\"}")
}

fn defer_cache_attr(tag_src: &str) -> Option<String> {
    for raw in tag_src.split_whitespace() {
        if let Some(rest) = raw.strip_prefix("cache=") {
            let value = rest.trim_matches(|c| {
                c == '"' || c == '\'' || c == '{' || c == '}' || c == '>' || c == '/'
            });
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Level of a §9.3 `server:defer` structural lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferLintLevel {
    Error,
    Warning,
}

/// A §9.3 structural finding from scanning the app/ tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferLint {
    pub level: DeferLintLevel,
    pub file: String,
    pub message: String,
}

/// Structural checks over the app/ tree (RFD 24 §9.3 amendment,
/// dekaruntime/rfd#46):
/// - ERROR: a `server:defer` island has no `slot="fallback"` child at all.
///   The spec makes fallback required; absence must fail the build instead
///   of silently rendering an empty default.
/// - WARNING: every child of a route's content region is deferred, so the
///   no-JS render shows only loading indicators.
pub fn scan_defer_lints(app_dir: &Path) -> Vec<DeferLint> {
    let mut lints: Vec<DeferLint> = scan_server_defer(app_dir)
        .into_iter()
        .filter(|d| !d.has_fallback)
        .map(|d| DeferLint {
            level: DeferLintLevel::Error,
            file: d.file.clone(),
            message: format!(
                "server:defer requires a child with slot=\"fallback\" ({})",
                d.component
            ),
        })
        .collect();
    if !app_dir.is_dir() {
        return lints;
    }
    let mut stack = vec![app_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(reader) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in reader.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if ext != "dsx" && ext != "ds" {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            let file = path.to_string_lossy().into_owned();
            let stripped = strip_ds_comments(&src);
            if returned_jsx_children_all_deferred(&stripped) {
                lints.push(DeferLint {
                    level: DeferLintLevel::Warning,
                    file: file.clone(),
                    message: format!(
                        "every child of the content region in {file} is deferred; the no-JS render shows only loading indicators"
                    ),
                });
            }
        }
    }
    lints
}

/// True when a `return <...>` block in `src` has children and every one of
/// them carries `server:defer`. Text or `{expression}` children count as
/// real content; whitespace between children is ignored.
fn returned_jsx_children_all_deferred(src: &str) -> bool {
    let mut from = 0;
    let mut saw_all_deferred = false;
    while let Some(rel) = src[from..].find("return <") {
        let at = from + rel + "return ".len();
        from = at + 1;
        let Some((body_start, body_end)) = jsx_element_body(src, at) else {
            continue;
        };
        if !jsx_children_all_deferred(&src[body_start..body_end]) {
            return false;
        }
        saw_all_deferred = true;
        from = body_end;
    }
    saw_all_deferred
}

/// Body range `(start, end)` of the JSX element whose `<` is at `start`.
/// Self-closed elements report an empty range.
fn jsx_element_body(src: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = src.as_bytes();
    if bytes.get(start) != Some(&b'<') || src[start..].starts_with("</") {
        return None;
    }
    let open_end = src[start..].find('>')? + start + 1;
    if src[start..open_end].trim_end().ends_with("/>") {
        return Some((open_end, open_end));
    }
    let mut depth = 1usize;
    let mut i = open_end;
    while i < bytes.len() {
        let rest = &src[i..];
        if rest.starts_with("</") {
            depth -= 1;
            if depth == 0 {
                return Some((open_end, i));
            }
            i += 2;
        } else if rest.starts_with('<') {
            let end = rest.find('>')? + i + 1;
            if !src[i..end].trim_end().ends_with("/>") {
                depth += 1;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    None
}

/// Index just past the JSX element whose `<` is at `start`.
fn skip_jsx_element(src: &str, start: usize) -> Option<usize> {
    jsx_element_body(src, start).and_then(|(body_start, body_end)| {
        if body_start == body_end {
            Some(body_start)
        } else {
            src[body_end..].find('>').map(|g| body_end + g + 1)
        }
    })
}

/// True when `body` has at least one child element and every direct child
/// element carries `server:defer` in its opening tag.
fn jsx_children_all_deferred(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut saw_child = false;
    while i < bytes.len() {
        match bytes[i] {
            b'<' if body[i..].starts_with("</") => return saw_child,
            b'<' => {
                let Some(grel) = body[i..].find('>') else {
                    return false;
                };
                let open_end = i + grel + 1;
                let open_tag = &body[i..open_end];
                if !open_tag.contains("server:defer") {
                    return false;
                }
                saw_child = true;
                i = if open_tag.trim_end().ends_with("/>") {
                    open_end
                } else {
                    skip_jsx_element(body, i).unwrap_or(open_end)
                };
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => return false,
        }
    }
    saw_child
}

/// §9.3 build diagnostics: ERROR lints fail the build; WARNING lints print
/// to stderr. Matches the `Err(String)` style of the other scan findings.
fn enforce_defer_lints(app_dir: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for lint in scan_defer_lints(app_dir) {
        match lint.level {
            DeferLintLevel::Warning => eprintln!("warning: {}", lint.message),
            DeferLintLevel::Error => errors.push(lint.message),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

pub fn defer_script_tag(has_defer: bool) -> String {
    if has_defer {
        "<script type=\"module\" src=\"/assets/islands-defer.js\"></script>".to_string()
    } else {
        String::new()
    }
}

pub fn write_defer_router_entry(project_root: &Path) -> Result<PathBuf, String> {
    let deferred = scan_server_defer(&project_root.join("app"));
    if deferred.is_empty() {
        return Err("no server:defer islands".to_string());
    }
    let missing: Vec<_> = deferred.iter().filter(|d| !d.has_fallback).collect();
    if !missing.is_empty() {
        let names = missing
            .iter()
            .map(|d| d.component.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "server:defer requires a child with slot=\"fallback\" ({names})"
        ));
    }
    let cache_dir = project_root.join(".cache").join("dekascript");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("defer-entry.dsx");
    let source = generate_defer_entry(&entry, project_root, &deferred)?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}

fn generate_defer_entry(
    entry: &Path,
    project_root: &Path,
    deferred: &[DeferredIsland],
) -> Result<String, String> {
    let mut imports = String::new();
    let mut registry = String::from("{ ");
    let mut seen = BTreeSet::new();
    let mut first = true;
    for (idx, item) in deferred.iter().enumerate() {
        let key = format!("{}:{}", item.file, item.component);
        if !seen.insert(key) {
            continue;
        }
        let alias = format!("Defer_{idx}");
        let rel = json_str(&pathdiff_dsx(entry, Path::new(&item.file)))?;
        let name = json_str(&item.component)?;
        imports.push_str(&format!(
            "import {{ {} as {alias} }} from {rel}\n",
            item.component
        ));
        if !first {
            registry.push_str(", ");
        }
        first = false;
        registry.push_str(&format!("{name}: {alias}"));
    }
    registry.push_str(" }");
    let cache_control = json_str(&format!("private, {}", defer_cache_header(deferred)))?;
    let secret = json_str(&ensure_defer_secret(project_root)?)?;
    let cookie = json_str(&session_cookie_name(project_root))?;
    Ok(format!(
        r#"{imports}import {{ runDeferBatch }} from "ui/server"

interface RequestHeaders {{ accept: string, cookie: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders, body: string }}
interface Response {{ status: number, body: string }}

async fn App(request: Request) Promise<Response> {{
    const boxed = unsafe {{ runDeferBatch(request.body, {secret}, {registry}, {cache_control}, request, {cookie}) }}
    const prom = match (boxed) {{
        Ok(p) => p,
        Err(_) => {{ status: 500, body: "Internal Server Error" }},
    }}
    return await prom
}}
export {{ App }}
"#
    ))
}

// Shared-cache `cache="60s"` shells reuse one encrypted marker. Do not put
// viewer-specific props on those islands; AAD binds name+cookie, not a login.
fn defer_cache_header(deferred: &[DeferredIsland]) -> String {
    let mut max_age: Option<u64> = None;
    for item in deferred {
        match item.cache.as_deref() {
            Some("no-store") | None => return "no-store".to_string(),
            Some(raw) => {
                let Some(secs) = raw.trim().trim_end_matches('s').parse::<u64>().ok() else {
                    return "no-store".to_string();
                };
                max_age = Some(max_age.map(|a| a.min(secs)).unwrap_or(secs));
            }
        }
    }
    match max_age {
        Some(secs) => format!("max-age={secs}"),
        None => "no-store".to_string(),
    }
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
    let islands = scan_client_islands(&app_dir);
    enforce_defer_lints(&app_dir)?;
    let deferred = scan_server_defer(&app_dir);
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
    if middleware_path(project_root).is_some() {
        write_middleware_router_entry(project_root)?;
    } else {
        let stale = project_root
            .join(".cache")
            .join("dekascript")
            .join("middleware-entry.ds");
        let _ = std::fs::remove_file(stale);
    }
    Ok(entry)
}

/// `.dsx` cannot import `api/`, so API dispatch lives in a sibling `.ds` file.
pub fn write_api_router_entry(project_root: &Path) -> Result<PathBuf, String> {
    let api_entries = scan_api_dir(&project_root.join("api"));
    if api_entries.is_empty() {
        return Err("no api/route.ds modules".to_string());
    }
    let cache_dir = project_root.join(".cache").join("dekascript");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("api-entry.ds");
    let source = generate_api_entry(&entry, &api_entries)?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}

pub fn middleware_path(project_root: &Path) -> Option<PathBuf> {
    let path = project_root.join(MIDDLEWARE_FILE);
    path.is_file().then_some(path)
}

pub fn project_needs_worker(project_root: &Path) -> bool {
    middleware_path(project_root).is_some()
        || !scan_api_dir(&project_root.join("api")).is_empty()
        || !scan_server_defer(&project_root.join("app")).is_empty()
}

pub fn request_path_from_url(url: &str) -> String {
    let without_query = url.split('?').next().unwrap_or(url);
    let path = if let Some(idx) = without_query.find("://") {
        let rest = &without_query[idx + 3..];
        rest.find('/').map(|i| &rest[i..]).unwrap_or("/")
    } else if without_query.starts_with('/') {
        without_query
    } else {
        "/"
    };
    collapse_leading_slashes(path)
}

fn collapse_leading_slashes(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    let trailing = path.len() > 1 && path.ends_with('/');
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let mut out = format!("/{trimmed}");
    if trailing && !out.ends_with('/') {
        out.push('/');
    }
    out
}

/// If the request path is not in canonical trailing-slash form, return the
/// Location value (path + query) to 301 to. Default is no trailing slash
/// except `/`.
pub fn trailing_slash_redirect_for_request(
    method: &str,
    url: &str,
    want_trailing: bool,
) -> Option<String> {
    if !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("HEAD") {
        return None;
    }
    let path = request_path_from_url(url);
    if path == "/api"
        || path.starts_with("/api/")
        || path == "/_deka/defer"
        || path.starts_with("/_deka/")
    {
        return None;
    }
    trailing_slash_redirect(url, want_trailing)
}

pub fn trailing_slash_redirect(url: &str, want_trailing: bool) -> Option<String> {
    let path = request_path_from_url(url);
    let query = url.split_once('?').map(|(_, q)| q);
    if path == "/" {
        return None;
    }
    let has_slash = path.ends_with('/');
    let dest = if want_trailing && !has_slash {
        format!("{path}/")
    } else if !want_trailing && has_slash {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        }
    } else {
        return None;
    };
    Some(match query {
        Some(q) => format!("{dest}?{q}"),
        None => dest,
    })
}

pub fn cloudflare_redirects(want_trailing: bool) -> &'static str {
    if want_trailing {
        // A splat add-slash rule loops (`/blog/` → `/blog//`). Serve and the
        // Worker own the add-slash 301 instead.
        "# Generated by deka build. Canonical: trailing slash (except /).\n# Add-slash is handled by the Worker / deka serve; a splat rule would loop.\n"
    } else {
        "# Generated by deka build. Canonical: no trailing slash (except /).\n/*/ /:splat 301\n"
    }
}

/// `None` means matcher is omitted (run on every app/api URL).
pub fn parse_middleware_matcher(source: &str) -> Option<Vec<String>> {
    let rest = source.split("export const matcher").nth(1)?;
    let start = rest.find('[')?;
    let end = rest[start..].find(']')?;
    let body = &rest[start + 1..start + end];
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    for ch in body.chars() {
        if ch == '"' {
            if in_str {
                out.push(std::mem::take(&mut cur));
                in_str = false;
            } else {
                in_str = true;
            }
            continue;
        }
        if in_str {
            cur.push(ch);
        }
    }
    Some(out)
}

pub fn matcher_hits(patterns: &[String], path: &str) -> bool {
    if patterns.is_empty() {
        return false;
    }
    patterns.iter().any(|pattern| pattern_hits(pattern, path))
}

pub fn pattern_hits(pattern: &str, path: &str) -> bool {
    let path = normalize_request_path(path);
    if let Some(prefix) = pattern.strip_suffix("/:path*") {
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    if let Some(prefix) = pattern.strip_suffix(":path*") {
        let prefix = prefix.trim_end_matches('/');
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    path == normalize_request_path(pattern)
}

pub fn skip_middleware_path(path: &str) -> bool {
    let path = normalize_request_path(path);
    path == "/assets" || path.starts_with("/assets/")
}

pub fn public_file_exists(project_root: &Path, path: &str) -> bool {
    let rel = path.trim_start_matches('/');
    if rel.is_empty() || rel.contains('\0') {
        return false;
    }
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return false;
    }
    let root = project_root.join("public");
    let file = root.join(rel_path);
    match (file.canonicalize(), root.canonicalize()) {
        (Ok(file), Ok(root)) => file.starts_with(&root) && file.is_file(),
        _ => file.is_file(),
    }
}

pub fn collect_public_rel_paths(project_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let root = project_root.join("public");
    collect_public_rel_paths_walk(&root, &root, &mut out);
    out.sort();
    out
}

fn collect_public_rel_paths_walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(reader) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in reader.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_public_rel_paths_walk(&path, base, out);
            continue;
        }
        if let Ok(rel) = path.strip_prefix(base) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !rel.is_empty() {
                out.push(format!("/{rel}"));
            }
        }
    }
}

/// Cookie whose value is AES-GCM AAD for deferred islands.
/// `serve.sessionCookie` in deka.json; default `deka_sid`. Empty string
/// disables session binding (anonymous AAD is just the component name).
fn session_cookie_name(project_root: &Path) -> String {
    let Ok(raw) = std::fs::read_to_string(project_root.join("deka.json")) else {
        return "deka_sid".to_string();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return "deka_sid".to_string();
    };
    match value
        .get("serve")
        .and_then(|serve| serve.get("sessionCookie").or_else(|| serve.get("session_cookie")))
        .and_then(|v| v.as_str())
    {
        Some(name) => name.to_string(),
        None => "deka_sid".to_string(),
    }
}

fn ensure_defer_secret(project_root: &Path) -> Result<String, String> {
    let cache_dir = project_root.join(".cache").join("dekascript");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let path = cache_dir.join("defer.key");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if trimmed.len() >= 32 {
            return Ok(trimmed.to_string());
        }
    }
    let secret = random_hex_32();
    std::fs::write(&path, secret.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    Ok(secret)
}

fn random_hex_32() -> String {
    let mut buf = [0u8; 32];
    #[cfg(unix)]
    {
        if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = file.read_exact(&mut buf);
        }
    }
    if buf.iter().all(|b| *b == 0) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(1);
        let mut state = nanos as u64 ^ std::process::id() as u64;
        for byte in &mut buf {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            *byte = (state >> 32) as u8;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// `.dsx` cannot import `.ds`, so middleware dispatch lives in a sibling file.
pub fn write_middleware_router_entry(project_root: &Path) -> Result<PathBuf, String> {
    let Some(mw) = middleware_path(project_root) else {
        return Err("no middleware.ds".to_string());
    };
    let cache_dir = project_root.join(".cache").join("dekascript");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("middleware-entry.ds");
    let source = generate_middleware_entry(&entry, &mw)?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}

/// Combined middleware + API graph compiled into `dist/_worker.js`.
pub fn write_worker_router_entry(project_root: &Path) -> Result<PathBuf, String> {
    let cache_dir = project_root.join(".cache").join("dekascript");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("worker-entry.ds");
    let source = generate_worker_entry(&entry, project_root)?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
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
        let head = with_css_links(css_plan, &page.route, head_concat(&manifest.entries, page, false))?;
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
    let not_found_head_inner = if manifest.not_found.as_ref().is_some_and(|e| exports_head(Path::new(&e.file))) {
        head_expr(&layout_head_aliases(&manifest.entries, "/"), "head_not_found()")
    } else {
        head_expr(&layout_head_aliases(&manifest.entries, "/"), "\"\"")
    };
    let not_found_head = with_css_links(css_plan, not_found_route, not_found_head_inner)?;

    let fallback_fn = if manifest.not_found.is_some() {
        String::new()
    } else {
        "fn FallbackNotFound() {\n    return <section><h1>Not found</h1></section>;\n}\n\n".to_string()
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

fn generate_api_entry(entry: &Path, api_entries: &[FrameworkEntry]) -> Result<String, String> {
    let (imports, registry) = api_imports_and_registry(entry, api_entries)?;
    Ok(format!(
        r#"{imports}import {{ runApiRouter }} from "ui/router"

interface RequestHeaders {{ accept: string }}
interface ResponseHeaders {{ location: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders, body: string }}
interface Response {{ status: number, body: string, headers: ResponseHeaders }}

fn App(request: Request) Response {{
    const boxed = unsafe {{ runApiRouter(request, {registry}) }}
    return match (boxed) {{
        Ok(r) => r,
        Err(_) => {{ status: 500, body: "Internal Server Error", headers: {{ location: "" }} }},
    }}
}}
export {{ App }}
"#
    ))
}

fn generate_middleware_entry(entry: &Path, middleware: &Path) -> Result<String, String> {
    let source = std::fs::read_to_string(middleware)
        .map_err(|err| format!("failed to read {}: {err}", middleware.display()))?;
    let matcher = parse_middleware_matcher(&source);
    let rel = json_str(&pathdiff_dsx(entry, middleware))?;
    let matcher_js = matcher_js_literal(&matcher)?;
    Ok(format!(
        r#"import {{ middleware }} from {rel}
import {{ runMiddleware }} from "ui/router"

interface RequestHeaders {{ accept: string }}
interface ResponseHeaders {{ location: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders }}
interface Response {{ status: number, body: string, headers: ResponseHeaders }}

fn App(request: Request) Response {{
    const boxed = unsafe {{ runMiddleware(request, middleware, {matcher_js}) }}
    return match (boxed) {{
        Ok(r) => r,
        Err(_) => {{ status: 500, body: "Internal Server Error", headers: {{ location: "" }} }},
    }}
}}
export {{ App }}
"#
    ))
}

fn matcher_js_literal(matcher: &Option<Vec<String>>) -> Result<String, String> {
    match matcher {
        None => Ok("null".to_string()),
        Some(patterns) => serde_json::to_string(patterns)
            .map_err(|err| format!("failed to encode matcher: {err}")),
    }
}

fn api_imports_and_registry(
    entry: &Path,
    api_entries: &[FrameworkEntry],
) -> Result<(String, String), String> {
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
    let mut registry = String::from("{ ");
    let mut first_route = true;
    for api in api_entries {
        let methods = exported_http_methods(Path::new(&api.file));
        if methods.is_empty() {
            continue;
        }
        assert_dynamic_route_supported(&api.route)?;
        let stem = alias("api", &api.route);
        for method in &methods {
            import_alias(&api.file, method, &format!("{method}_{stem}"))?;
        }
        if !first_route {
            registry.push_str(", ");
        }
        first_route = false;
        registry.push_str(&format!("{}: {{ ", json_str(&api.route)?));
        for (i, method) in methods.iter().enumerate() {
            if i > 0 {
                registry.push_str(", ");
            }
            registry.push_str(&format!("{method}: {method}_{stem}"));
        }
        registry.push_str(" }");
    }
    registry.push_str(" }");
    Ok((imports, registry))
}

fn generate_worker_entry(entry: &Path, project_root: &Path) -> Result<String, String> {
    let mut imports = String::new();
    let mw = middleware_path(project_root);
    let mut mw_arg = "null".to_string();
    let mut matcher_js = "null".to_string();
    if let Some(mw) = &mw {
        let rel = json_str(&pathdiff_dsx(entry, mw))?;
        imports.push_str(&format!("import {{ middleware }} from {rel}\n"));
        mw_arg = "middleware".to_string();
        let matcher = std::fs::read_to_string(mw)
            .ok()
            .and_then(|src| parse_middleware_matcher(&src));
        matcher_js = matcher_js_literal(&matcher)?;
    }
    let api_entries = scan_api_dir(&project_root.join("api"));
    let (api_imports, registry) = api_imports_and_registry(entry, &api_entries)?;
    imports.push_str(&api_imports);
    Ok(format!(
        r#"{imports}import {{ runWorker }} from "ui/router"

interface RequestHeaders {{ accept: string }}
interface ResponseHeaders {{ location: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders, body: string }}
interface Response {{ status: number, body: string, headers: ResponseHeaders }}

fn App(request: Request) Response {{
    const boxed = unsafe {{ runWorker(request, {mw_arg}, {matcher_js}, {registry}) }}
    return match (boxed) {{
        Ok(r) => r,
        Err(_) => {{ status: 500, body: "Internal Server Error", headers: {{ location: "" }} }},
    }}
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

fn json_str(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| format!("failed to encode string: {err}"))
}

fn with_css_links(plan: &CssPlan, route: &str, head_js: String) -> Result<String, String> {
    let links = css_links_for_route(plan, route);
    if links.is_empty() {
        return Ok(head_js);
    }
    Ok(format!("{} + {}", json_str(&links)?, head_js))
}

fn exports_head(path: &Path) -> bool {
    let Ok(src) = std::fs::read_to_string(path) else {
        return false;
    };
    exports_fn_named(&strip_ds_comments(&src), "head")
}

fn alias(prefix: &str, route: &str) -> String {
    let mut out = prefix.to_string();
    if route == "/" {
        out.push_str("_root");
        return out;
    }
    out.push('_');
    out.push_str(&ident_slug(route));
    out
}

fn ident_slug(route: &str) -> String {
    let mut out = String::new();
    for ch in route.trim_matches('/').chars() {
        match ch {
            '/' => out.push_str("_s_"),
            '_' => out.push_str("_u_"),
            '-' => out.push_str("_h_"),
            '[' => out.push_str("_l_"),
            ']' => out.push_str("_r_"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ => out.push_str("_x_"),
        }
    }
    if out.is_empty() {
        "root".to_string()
    } else {
        out
    }
}

fn assert_dynamic_route_supported(route: &str) -> Result<(), String> {
    let parts: Vec<&str> = route
        .trim_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    let dynamic: Vec<(usize, &str)> = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.starts_with('['))
        .map(|(i, part)| (i, *part))
        .collect();
    if dynamic.len() > 1 {
        return Err(format!(
            "v1 dynamic routes support one [param] at the end ({route})"
        ));
    }
    if dynamic.len() == 1 && dynamic[0].0 != parts.len() - 1 {
        return Err(format!(
            "v1 dynamic routes require [param] as the last segment ({route})"
        ));
    }
    Ok(())
}

fn path_condition(route: &str) -> Result<String, String> {
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

fn ancestor_routes(route: &str) -> Vec<String> {
    if route == "/" {
        return vec!["/".to_string()];
    }
    let parts: Vec<&str> = route.trim_matches('/').split('/').filter(|p| !p.is_empty()).collect();
    let mut out = Vec::new();
    out.push(if route.starts_with('/') {
        route.to_string()
    } else {
        format!("/{route}")
    });
    for i in (0..parts.len().saturating_sub(1)).rev() {
        out.push(format!("/{}", parts[..=i].join("/")));
    }
    out.push("/".to_string());
    out
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

fn page_call(route: &str, page_alias: &str) -> String {
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
            route_from_relative_path(FrameworkEntryKind::Page, "page.dsx"),
            Some("/".to_string())
        );
    }

    #[test]
    fn derives_nested_page_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "users/page.dsx"),
            Some("/users".to_string())
        );
    }

    #[test]
    fn derives_layout_scope_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Layout, "dashboard/layout.dsx"),
            Some("/dashboard".to_string())
        );
    }

    #[test]
    fn derives_api_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Api, "api/packages.dsx"),
            Some("/api/packages".to_string())
        );
    }

    #[test]
    fn preserves_dynamic_segments() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "blog/[slug]/page.dsx"),
            Some("/blog/[slug]".to_string())
        );
    }

    #[test]
    fn rejects_phpx_entry_files() {
        // RFD 24 §12: `.phpx` files are no longer framework entries.
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "page.phpx"),
            None
        );
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Layout, "dashboard/layout.phpx"),
            None
        );
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Api, "api/packages.phpx"),
            None
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
        assert_eq!(page_call("/", "Page_root"), "<Page_root />");
        assert_eq!(
            page_call("/blog/[slug]", "Page_blog__slug_"),
            "<Page_blog__slug_ slug={last_segment(path)} />"
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
            source.contains("fn title_from_head(headHtml: string) string"),
            "generated entry should define title_from_head: {source}"
        );
        assert!(
            source.contains("title: title_from_head(headHtml)"),
            "fragment payload must include the merged title (RFD 24 §8.4): {source}"
        );
        assert!(
            source.contains("last_segment"),
            "generated entry should define last_segment: {source}"
        );
        assert!(
            source.contains("slug={last_segment(path)}"),
            "generated [slug] page call should pass params: {source}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn path_condition_escapes_quotes_in_route() {
        let cond = path_condition("/foo\"bar").expect("escape route");
        assert!(
            cond.contains("\\\"") || cond.contains("\\u0022"),
            "route with a quote must be escaped: {cond}"
        );
        assert!(
            !cond.contains("path == \"/foo\"bar\""),
            "unescaped quote would break generated source: {cond}"
        );
    }

    #[test]
    fn matcher_hits_exact_and_path_star() {
        assert!(pattern_hits("/dashboard", "/dashboard"));
        assert!(!pattern_hits("/dashboard", "/dashboard/settings"));
        assert!(pattern_hits("/dashboard/:path*", "/dashboard"));
        assert!(pattern_hits("/dashboard/:path*", "/dashboard/settings"));
        assert!(!pattern_hits("/dashboard/:path*", "/login"));
        assert!(matcher_hits(
            &["/admin".to_string(), "/dashboard/:path*".to_string()],
            "/dashboard/x"
        ));
        assert!(!matcher_hits(&["/admin".to_string()], "/dashboard"));
        assert!(!matcher_hits(&[], "/dashboard"));
    }

    #[test]
    fn parse_matcher_reads_string_array() {
        let src = "export const matcher: array<string> = [\"/dashboard/:path*\", \"/admin\"]\nexport fn middleware() { return None }\n";
        assert_eq!(
            parse_middleware_matcher(src),
            Some(vec![
                "/dashboard/:path*".to_string(),
                "/admin".to_string()
            ])
        );
        assert_eq!(parse_middleware_matcher("export fn middleware() { return None }\n"), None);
    }

    #[test]
    fn trailing_slash_redirect_strips_by_default() {
        assert_eq!(
            trailing_slash_redirect("http://localhost/blog/", false),
            Some("/blog".to_string())
        );
        assert_eq!(trailing_slash_redirect("http://localhost/blog", false), None);
        assert_eq!(trailing_slash_redirect("http://localhost/", false), None);
        assert_eq!(
            trailing_slash_redirect("http://localhost/blog?x=1", true),
            Some("/blog/?x=1".to_string())
        );
    }

    #[test]
    fn scan_api_dir_maps_route_files() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_api_scan_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("hello")).unwrap();
        std::fs::write(
            tmp.join("hello/route.ds"),
            "export fn GET(request: Request) Response { return { status: 200, body: \"ok\" } }\n",
        )
        .unwrap();
        let entries = scan_api_dir(&tmp);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].route, "/api/hello");
        let methods = exported_http_methods(Path::new(&entries[0].file));
        assert_eq!(methods, vec!["GET".to_string()]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn exported_http_methods_uses_word_boundary_and_skips_comments() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_api_methods_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("route.ds");
        std::fs::write(
            &path,
            "// export fn DELETE(request: Request) Response { return { status: 200, body: \"no\" } }\nexport fn GETTER() { return 1 }\nexport fn GET(request: Request) Response { return { status: 200, body: \"ok\" } }\n",
        )
        .unwrap();
        let methods = exported_http_methods(&path);
        assert_eq!(methods, vec!["GET".to_string()]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn skip_middleware_for_assets_and_public_files() {
        assert!(skip_middleware_path("/assets/app.js"));
        assert!(!skip_middleware_path("/dashboard"));
        let tmp = std::env::temp_dir().join(format!(
            "deka_pub_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("public")).unwrap();
        std::fs::write(tmp.join("public/style.css"), "body{}").unwrap();
        assert!(public_file_exists(&tmp, "/style.css"));
        assert!(!public_file_exists(&tmp, "/missing.css"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn wrap_layouts_desugars_loading_around_child_segment() {
        let entries = vec![
            FrameworkEntry {
                kind: FrameworkEntryKind::Layout,
                route: "/".into(),
                file: "app/layout.dsx".into(),
            },
            FrameworkEntry {
                kind: FrameworkEntryKind::Loading,
                route: "/".into(),
                file: "app/loading.dsx".into(),
            },
            FrameworkEntry {
                kind: FrameworkEntryKind::Layout,
                route: "/blog".into(),
                file: "app/blog/layout.dsx".into(),
            },
            FrameworkEntry {
                kind: FrameworkEntryKind::Loading,
                route: "/blog".into(),
                file: "app/blog/loading.dsx".into(),
            },
            FrameworkEntry {
                kind: FrameworkEntryKind::Page,
                route: "/blog".into(),
                file: "app/blog/page.dsx".into(),
            },
        ];
        let tree = wrap_layouts(&entries, "/blog", "Page_blog");
        assert!(
            tree.contains("<Suspense fallback={<Loading_blog />}><Page_blog /></Suspense>"),
            "blog loading wraps the page, not the blog layout: {tree}"
        );
        assert!(
            tree.contains("<Layout_root>") && tree.contains("<Suspense fallback={<Loading_root />}"),
            "root loading wraps the child of the root layout: {tree}"
        );
        assert!(
            !tree.starts_with("<Suspense"),
            "root loading must not wrap the root layout chrome: {tree}"
        );
    }

    #[test]
    fn wrap_layouts_applies_loading_without_a_layout_at_that_segment() {
        let entries = vec![
            FrameworkEntry {
                kind: FrameworkEntryKind::Layout,
                route: "/".into(),
                file: "app/layout.dsx".into(),
            },
            FrameworkEntry {
                kind: FrameworkEntryKind::Loading,
                route: "/blog".into(),
                file: "app/blog/loading.dsx".into(),
            },
            FrameworkEntry {
                kind: FrameworkEntryKind::Page,
                route: "/blog/post".into(),
                file: "app/blog/post/page.dsx".into(),
            },
        ];
        let tree = wrap_layouts(&entries, "/blog/post", "Page_blog_post");
        assert!(
            tree.contains("<Suspense fallback={<Loading_blog />}><Page_blog_post /></Suspense>"),
            "blog loading.dsx must wrap the child even without blog/layout.dsx: {tree}"
        );
    }

    #[test]
    fn generated_serve_entry_imports_suspense_for_loading() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_loading_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("app")).unwrap();
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
            "export fn Page() {\n    return <section><h1>Home</h1></section>;\n}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("app/loading.dsx"),
            "export fn Loading() {\n    return <p>Loading...</p>;\n}\n",
        )
        .unwrap();
        let entry = write_app_router_entry(&tmp).expect("generate serve-entry");
        let source = std::fs::read_to_string(&entry).expect("read serve-entry");
        assert!(
            source.contains("import { Suspense } from \"ui/suspense\""),
            "generated entry should import Suspense: {source}"
        );
        assert!(
            source.contains("<Suspense fallback={<Loading_root />}"),
            "generated entry should desugar loading.dsx: {source}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scan_client_islands_finds_directive_and_props() {
        let src = "export fn Page() {\n    return <Cart client:load userId={id} />;\n}\n";
        let found = islands_in_source(src, "app/page.dsx");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].component, "Cart");
        assert_eq!(found[0].directive, "load");
        assert!(found[0].props.contains(&"userId".to_string()));
        let tags = island_script_tags(&found);
        assert!(tags.contains("islands-load.js"));
        assert!(!tags.contains("islands-idle.js"));
    }

    #[test]
    fn scan_client_islands_ignores_server_defer() {
        let src = "export fn Page() {\n    return <Cart server:defer userId={id} />;\n}\n";
        let found = islands_in_source(src, "app/page.dsx");
        assert!(found.is_empty());
        assert_eq!(island_script_tags(&found), "");
    }

    #[test]
    fn scan_server_defer_requires_fallback_slot() {
        let missing = defer_in_source(
            "export fn Page() {\n    return <Cart server:defer userId={id} />;\n}\n",
            "app/page.dsx",
        );
        assert_eq!(missing.len(), 1);
        assert!(!missing[0].has_fallback);
        let ok = defer_in_source(
            "export fn Page() {\n    return <Cart server:defer cache=\"60s\"><span slot=\"fallback\">.</span></Cart>;\n}\n",
            "app/page.dsx",
        );
        assert_eq!(ok.len(), 1);
        assert!(ok[0].has_fallback);
        assert_eq!(ok[0].component, "Cart");
        assert_eq!(ok[0].cache.as_deref(), Some("60s"));
        assert!(defer_script_tag(true).contains("islands-defer.js"));
    }

    #[test]
    fn defer_lints_error_without_fallback_and_pass_with_fallback() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_defer_lint_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("app")).unwrap();
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <main><h1>Hi</h1></main>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        assert!(
            lints.iter().all(|l| l.level != DeferLintLevel::Error),
            "static content must not error: {lints:?}"
        );
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <Cart server:defer />;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        let errors: Vec<_> = lints
            .iter()
            .filter(|l| l.level == DeferLintLevel::Error)
            .collect();
        assert_eq!(errors.len(), 1, "{lints:?}");
        assert!(
            errors[0].message.contains("slot=\"fallback\""),
            "{}",
            errors[0].message
        );
        assert!(errors[0].message.contains("Cart"), "{}", errors[0].message);
        // Negative case: a defer WITH fallback must not error.
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <Cart server:defer><span slot=\"fallback\">.</span></Cart>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        assert!(
            lints.iter().all(|l| l.level != DeferLintLevel::Error),
            "defer with fallback must not error: {lints:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn defer_lints_warn_when_every_content_child_is_deferred() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_defer_warn_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("app")).unwrap();
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <main><A server:defer><i slot=\"fallback\">a</i></A><B server:defer><i slot=\"fallback\">b</i></B></main>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        let warnings: Vec<_> = lints
            .iter()
            .filter(|l| l.level == DeferLintLevel::Warning)
            .collect();
        assert_eq!(warnings.len(), 1, "{lints:?}");
        assert!(
            warnings[0].message.contains("every child of the content region"),
            "{}",
            warnings[0].message
        );
        // Mixed content: no warning.
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <main><h1>Hi</h1><A server:defer><i slot=\"fallback\">a</i></A></main>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        assert!(
            lints.iter().all(|l| l.level != DeferLintLevel::Warning),
            "mixed content must not warn: {lints:?}"
        );
        // Empty content region: no warning either.
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <main></main>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        assert!(
            lints.iter().all(|l| l.level != DeferLintLevel::Warning),
            "empty content region must not warn: {lints:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn app_router_entry_fails_build_when_defer_lacks_fallback() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_defer_build_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("app")).unwrap();
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
            "export fn Page() {\n    return <Cart server:defer />;\n}\n",
        )
        .unwrap();
        let err = write_app_router_entry(&tmp)
            .expect_err("server:defer without fallback must fail the build");
        assert!(
            err.contains("slot=\"fallback\""),
            "error should name the missing fallback: {err}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn island_script_tags_empty_without_islands() {
        assert_eq!(island_script_tags(&[]), "");
    }

    #[test]
    fn collect_class_literals_reads_string_attrs() {
        let src = r#"return <div class="p-4 text-lg"><span class={'bg-white'}>x</span></div>;"#;
        let classes = collect_class_literals(src);
        assert!(classes.contains("p-4"));
        assert!(classes.contains("text-lg"));
        assert!(classes.contains("bg-white"));
    }

    #[test]
    fn css_plan_hoists_shared_classes() {
        let styles = vec![
            RouteStyle {
                route: "/".into(),
                classes: ["p-4", "text-lg"].into_iter().map(str::to_string).collect(),
                files: vec![],
            },
            RouteStyle {
                route: "/about".into(),
                classes: ["p-4", "text-sm"].into_iter().map(str::to_string).collect(),
                files: vec![],
            },
        ];
        let plan = css_plan_from_styles(&styles);
        assert!(plan.common);
        assert_eq!(plan.routes.get("/"), Some(&true));
        assert_eq!(plan.routes.get("/about"), Some(&true));
        let home = css_links_for_route(&plan, "/");
        assert!(home.contains("/assets/css/common.css"));
        assert!(home.contains("/assets/css/route-root.css"));
        let about = css_links_for_route(&plan, "/about");
        assert!(about.contains("route-about.css"));
    }

    #[test]
    fn css_plan_emits_common_for_a_single_route() {
        let styles = vec![RouteStyle {
            route: "/".into(),
            classes: ["p-4"].into_iter().map(str::to_string).collect(),
            files: vec![],
        }];
        let plan = css_plan_from_styles(&styles);
        assert!(plan.common, "preflight/common.css must exist for a one-page app");
        assert!(css_links_for_route(&plan, "/").contains("/assets/css/common.css"));
    }

    #[test]
    fn route_css_slug_does_not_collide() {
        let a = route_css_slug("/a/b");
        let b = route_css_slug("/a_b");
        let c = route_css_slug("/a-b");
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
        assert_ne!(route_css_slug("/blog/[id]"), route_css_slug("/blog/[slug]"));
    }

    #[test]
    fn alias_uses_ident_slug_so_paths_do_not_collide() {
        assert_ne!(alias("Page", "/a/b"), alias("Page", "/a_b"));
        assert_ne!(alias("Page", "/a-b"), alias("Page", "/a_b"));
        assert!(
            alias("Page", "/blog/[slug]")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "generated import aliases must be identifiers: {}",
            alias("Page", "/blog/[slug]")
        );
    }

    #[test]
    fn dynamic_routes_must_be_a_single_trailing_param() {
        assert!(path_condition("/blog/[slug]").is_ok());
        assert!(path_condition("/blog/[id]/comments").is_err());
        assert!(path_condition("/[a]/[b]").is_err());
    }

    #[test]
    fn trailing_slash_redirect_skips_post_and_api() {
        assert_eq!(
            trailing_slash_redirect_for_request("POST", "http://localhost/_deka/defer", true),
            None
        );
        assert_eq!(
            trailing_slash_redirect_for_request("POST", "http://localhost/api/hello/", false),
            None
        );
        assert_eq!(
            trailing_slash_redirect_for_request("GET", "http://localhost/blog/", false),
            Some("/blog".to_string())
        );
        assert_eq!(
            trailing_slash_redirect_for_request("HEAD", "http://localhost/blog/", false),
            Some("/blog".to_string())
        );
    }

    #[test]
    fn scan_skips_commented_island_and_defer_directives() {
        let src = "// <Cart client:load />\n/* <Badge server:defer></Badge> */\nexport fn Page() { return <div /> }\n";
        assert!(islands_in_source(src, "app/page.dsx").is_empty());
        assert!(defer_in_source(src, "app/page.dsx").is_empty());
    }

    #[test]
    fn trailing_slash_redirect_does_not_open_redirect() {
        assert_eq!(
            trailing_slash_redirect("//evil.com/foo/", false),
            Some("/evil.com/foo".to_string())
        );
        assert_eq!(
            trailing_slash_redirect("http://localhost//evil.com/foo/", false),
            Some("/evil.com/foo".to_string())
        );
        let dest = trailing_slash_redirect("//evil.com/foo", true).unwrap();
        assert!(dest.starts_with('/'));
        assert!(!dest.starts_with("//"));
    }

    #[test]
    fn cloudflare_redirects_true_does_not_emit_looping_splat() {
        let rules = cloudflare_redirects(true);
        assert!(!rules.contains("/* /:splat/"));
        assert!(cloudflare_redirects(false).contains("/*/ /:splat 301"));
    }

    #[test]
    fn skip_middleware_does_not_bypass_deka_defer() {
        assert!(!skip_middleware_path("/_deka/defer"));
        assert!(skip_middleware_path("/assets/islands-load.js"));
    }

    #[test]
    fn public_file_exists_rejects_traversal() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_pub_trav_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("public")).unwrap();
        std::fs::write(tmp.join("secret.env"), "nope").unwrap();
        std::fs::write(tmp.join("public/ok.css"), "body{}").unwrap();
        assert!(public_file_exists(&tmp, "/ok.css"));
        assert!(!public_file_exists(&tmp, "/../secret.env"));
        assert!(!public_file_exists(&tmp, "//secret.env"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn exports_head_does_not_match_header() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_head_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("page.dsx");
        std::fs::write(&path, "export fn header() { return 1 }\n").unwrap();
        assert!(!exports_head(&path));
        std::fs::write(&path, "export fn head() { return <title>x</title> }\n").unwrap();
        assert!(exports_head(&path));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn exported_http_methods_sees_async_and_export_list() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_api_async_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("route.ds");
        std::fs::write(
            &path,
            "async fn GET(request: Request) Promise<Response> { return { status: 200, body: \"ok\" } }\nexport { GET }\n",
        )
        .unwrap();
        assert_eq!(exported_http_methods(&path), vec!["GET".to_string()]);
        std::fs::write(
            &path,
            "export async fn POST(request: Request) Promise<Response> { return { status: 200, body: \"ok\" } }\n",
        )
        .unwrap();
        assert_eq!(exported_http_methods(&path), vec!["POST".to_string()]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn collect_import_paths_reads_multiline_from() {
        let src = "import {\n  Card\n} from \"./Card.dsx\"\nimport \"./theme.css\"\n";
        let ds = collect_import_paths(src, |p| p.ends_with(".dsx"));
        assert_eq!(ds, vec!["./Card.dsx".to_string()]);
        let css = collect_import_paths(src, |p| p.ends_with(".css"));
        assert_eq!(css, vec!["./theme.css".to_string()]);
    }

    #[test]
    fn generate_defer_entry_returns_async_iife_and_caps_batch() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_defer_gen_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("app")).unwrap();
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Badge() { return <strong>42</strong> }\nexport fn Page() { return <Badge server:defer><span slot=\"fallback\">.</span></Badge> }\n",
        )
        .unwrap();
        let entry = write_defer_router_entry(&tmp).expect("write defer-entry");
        let source = std::fs::read_to_string(&entry).expect("read defer-entry");
        assert!(source.contains("runDeferBatch"));
        assert!(source.contains("import { runDeferBatch } from \"ui/server\""));
        assert!(source.contains("runDeferBatch(request.body"));
        assert!(source.contains("\"deka_sid\""), "{source}");
        assert!(!source.contains("headers: { \\\"cache-control\\\""));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn session_cookie_name_reads_serve_config() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_sid_cfg_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(session_cookie_name(&tmp), "deka_sid");
        std::fs::write(
            tmp.join("deka.json"),
            "{ \"serve\": { \"sessionCookie\": \"sid\" } }\n",
        )
        .unwrap();
        assert_eq!(session_cookie_name(&tmp), "sid");
        std::fs::write(
            tmp.join("deka.json"),
            "{ \"serve\": { \"sessionCookie\": \"\" } }\n",
        )
        .unwrap();
        assert_eq!(session_cookie_name(&tmp), "");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn middleware_entry_calls_run_middleware() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_mw_gen_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("middleware.ds"),
            "export const matcher = [\"/_deka/defer\"]\nexport fn middleware(request: Request) Option<Response> { return None }\n",
        )
        .unwrap();
        let entry = write_middleware_router_entry(&tmp).expect("write middleware-entry");
        let source = std::fs::read_to_string(&entry).expect("read middleware-entry");
        assert!(
            source.contains("import { runMiddleware } from \"ui/router\""),
            "{source}"
        );
        assert!(source.contains("runMiddleware(request, middleware,"), "{source}");
        assert!(
            !source.contains("opt.__case"),
            "Option lowering belongs in ui/router, not generated text: {source}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
