//! CSS collection: walks each route's module graph for `class="..."`
//! literals and component-scoped `.css` imports, then plans common vs
//! per-route stylesheets (RFD 24 §10.6).
//!
//! This lives in `runtime_core`, not `runtime::css`, because
//! [`css_scope_hash`] is pinned against `deka_emit::css_scope_hash` and
//! `deka_emit` cannot depend on the `runtime` crate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::module_spec::ds_source_candidates;

use super::manifest::{FrameworkEntry, FrameworkEntryKind, FrameworkManifest};
use super::routes::{layout_chain, route_css_slug};
use super::source::{collect_import_paths, resolve_relative};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteStyle {
    pub route: String,
    pub classes: BTreeSet<String>,
    pub files: Vec<ScopedStyleFile>,
}

/// A component-authored CSS file and the style-scope id of the module that
/// imports it. The CSS writer rewrites the file's selectors to require
/// `[data-deka-cid-<cid>]`; the compiler stamps the importing module's host
/// elements with the same attribute (RFD 24 §10.6). A shared CSS file can
/// appear once per importing module, each with its own cid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedStyleFile {
    pub path: String,
    pub cid: String,
}

/// FNV-1a 64-bit over the module source, truncated to 12 hex chars — the
/// `data-deka-cid-<hash>` component style-scope id from RFD 24 §10.6.
///
/// Must stay in sync with `deka_emit::css_scope_hash`: the compiler stamps
/// elements with this id while the CSS writer rewrites selectors with it, so
/// both sides must produce the same digest. Both crates pin the same test
/// vector.
pub fn css_scope_hash(source: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:012x}", hash & 0xffff_ffff_ffff)
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CssPlan {
    pub common: bool,
    pub routes: BTreeMap<String, bool>,
    pub common_classes: BTreeSet<String>,
    pub common_files: BTreeSet<String>,
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
        files.extend(
            layout_chain(&manifest.entries, &page.route)
                .into_iter()
                .map(|e| e.file.clone()),
        );
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
        files.extend(
            layout_chain(&manifest.entries, "/")
                .into_iter()
                .map(|e| e.file.clone()),
        );
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
            if seen.insert(file.path.clone()) {
                *file_count.entry(file.path.clone()).or_insert(0) += 1;
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
        let unique_file = style.files.iter().any(|f| !common_files.contains(&f.path));
        routes.insert(style.route.clone(), unique_class || unique_file);
    }
    CssPlan {
        common,
        routes,
        common_classes,
        common_files,
    }
}
fn scan_style_graph(entry_files: &[String]) -> (BTreeSet<String>, Vec<ScopedStyleFile>) {
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
        // Every CSS file discovered in this module is scoped to it: the
        // compiler stamps this module's host elements with
        // `[data-deka-cid-<cid>]`, so the rewritten selectors must carry the
        // same id (RFD 24 §10.6).
        let cid = css_scope_hash(&src);
        for spec in collect_import_paths(&src, |p| p.starts_with("./") || p.starts_with("../")) {
            let Some(base) = resolve_relative(path, &spec) else {
                continue;
            };
            // Shared resolver (deka#241 / #622). Do not filter on `.ds`/`.dsx`
            // before this call: extensionless `./Card` is a real specifier.
            let candidates = ds_source_candidates(&base);
            if candidates.is_empty() {
                // `.css` is not DekaScript source; candidates is empty by design.
                if spec.to_ascii_lowercase().ends_with(".css") && base.is_file() {
                    let resolved = base.to_string_lossy().into_owned();
                    let key = format!("{resolved}\u{0}{cid}");
                    if seen_css.insert(key) {
                        css_files.push(ScopedStyleFile {
                            path: resolved,
                            cid: cid.clone(),
                        });
                    }
                }
                continue;
            }
            if let Some(resolved) = candidates.into_iter().find(|p| p.is_file()) {
                stack.push(resolved.to_string_lossy().into_owned());
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

#[cfg(test)]
mod tests {
    use super::super::manifest::scan_app_dir;
    use super::*;

    #[test]
    fn collect_class_literals_reads_string_attrs() {
        let src = r#"return <div class="p-4 text-lg"><span class={'bg-white'}>x</span></div>;"#;
        let classes = collect_class_literals(src);
        assert!(classes.contains("p-4"));
        assert!(classes.contains("text-lg"));
        assert!(classes.contains("bg-white"));
    }

    #[test]
    fn css_scope_hash_matches_the_emitter() {
        // Pinned vector shared with deka_emit::css_scope_hash — the compiler
        // stamps elements with this id and the CSS writer rewrites selectors
        // with it, so a drift between the two crates breaks every scope.
        assert_eq!(css_scope_hash("greeting {}"), "e1c9193cd172");
        assert_eq!(css_scope_hash("greeting {}"), css_scope_hash("greeting {}"));
        assert_ne!(
            css_scope_hash("greeting {}"),
            css_scope_hash("greeting { }")
        );
    }

    #[test]
    fn scan_style_graph_scopes_css_to_the_importing_module() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_css_scan_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let card_src =
            "import \"./card.css\"\nexport fn Card() { return <div class=\"card\">x</div> }\n";
        let page_src = "import \"./Card.dsx\"\nimport \"./page.css\"\nexport fn Page() { return <main class=\"hero\"><Card /></main> }\n";
        std::fs::write(tmp.join("Card.dsx"), card_src).unwrap();
        std::fs::write(tmp.join("page.dsx"), page_src).unwrap();
        std::fs::write(tmp.join("card.css"), ".card { color: red; }").unwrap();
        std::fs::write(tmp.join("page.css"), ".hero { color: blue; }").unwrap();

        let (classes, files) =
            scan_style_graph(&[tmp.join("page.dsx").to_string_lossy().to_string()]);
        assert!(classes.contains("card"), "classes: {classes:?}");
        assert!(classes.contains("hero"), "classes: {classes:?}");

        let card_css = files
            .iter()
            .find(|f| f.path.ends_with("card.css"))
            .expect("card.css in the style graph");
        assert_eq!(
            card_css.cid,
            css_scope_hash(card_src),
            "CSS is scoped to the module that imports it"
        );
        let page_css = files
            .iter()
            .find(|f| f.path.ends_with("page.css"))
            .expect("page.css in the style graph");
        assert_eq!(page_css.cid, css_scope_hash(page_src));

        // Same-named modules with different sources must not share a scope.
        assert_ne!(card_css.cid, page_css.cid);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn collect_route_styles_buckets_layout_and_page_scopes() {
        let tmp = std::env::temp_dir().join(format!(
            "deka_css_routes_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(tmp.join("app/blog")).unwrap();
        let layout_src =
            "import \"./layout.css\"\nexport fn Layout() { return <div class=\"shell\">x</div> }\n";
        let page_src = "import \"./page.css\"\nexport fn Page() { return <article class=\"post\">x</article> }\n";
        std::fs::write(tmp.join("app/layout.dsx"), layout_src).unwrap();
        std::fs::write(tmp.join("app/blog/page.dsx"), page_src).unwrap();
        std::fs::write(tmp.join("app/layout.css"), ".shell { margin: 0; }").unwrap();
        std::fs::write(tmp.join("app/blog/page.css"), ".post { margin: 0; }").unwrap();

        let manifest = scan_app_dir(&tmp.join("app"));
        let styles = collect_route_styles(&manifest);
        let blog = styles
            .iter()
            .find(|s| s.route == "/blog")
            .expect("/blog route styles");
        let cids: Vec<&str> = blog.files.iter().map(|f| f.cid.as_str()).collect();
        assert!(
            cids.contains(&css_scope_hash(layout_src).as_str()),
            "layout CSS rides the nested layout's own scope: {cids:?}"
        );
        assert!(
            cids.contains(&css_scope_hash(page_src).as_str()),
            "page CSS rides the page's own scope: {cids:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
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
        assert!(
            plan.common,
            "preflight/common.css must exist for a one-page app"
        );
        assert!(css_links_for_route(&plan, "/").contains("/assets/css/common.css"));
    }
}
