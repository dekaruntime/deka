//! Route parsing, slugs, and segments: relative paths → routes, route →
//! pattern matching, layout chains, dynamic segments, and the slug encodings
//! used for import aliases and per-route CSS files. Pure data in → data out;
//! zero compiler dependencies.

use std::collections::BTreeMap;

use super::manifest::{FrameworkEntry, FrameworkEntryKind, FrameworkManifest};
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

pub(super) fn dynamic_rank(route: &str) -> (u8, usize) {
    let dynamic = route.contains('[') as u8;
    (dynamic, usize::MAX - route.len())
}
pub fn route_pattern_matches(pattern: &str, path: &str) -> Option<BTreeMap<String, String>> {
    let pat: Vec<&str> = pattern
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let seg: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
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
            // Hex-encode every other char so distinct routes cannot collide
            // in the fallback (plain `-x-` mapped both `/a.b` and `/a b` to
            // `a-x-b`). `-` never appears raw in the output, so the encoding
            // stays unambiguous.
            c => out.push_str(&format!("-x{:x}-", c as u32)),
        }
    }
    if out.is_empty() {
        "root".to_string()
    } else {
        out
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
pub fn cloudflare_redirects(want_trailing: bool) -> &'static str {
    if want_trailing {
        // A splat add-slash rule loops (`/blog/` → `/blog//`). Serve and the
        // Worker own the add-slash 301 instead.
        "# Generated by deka build. Canonical: trailing slash (except /).\n# Add-slash is handled by the Worker / deka serve; a splat rule would loop.\n"
    } else {
        "# Generated by deka build. Canonical: no trailing slash (except /).\n/*/ /:splat 301\n"
    }
}
pub(super) fn ident_slug(route: &str) -> String {
    let mut out = String::new();
    for ch in route.trim_matches('/').chars() {
        match ch {
            '/' => out.push_str("_s_"),
            '_' => out.push_str("_u_"),
            '-' => out.push_str("_h_"),
            '[' => out.push_str("_l_"),
            ']' => out.push_str("_r_"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ => out.push_str(&format!("_x{:x}_", ch as u32)),
        }
    }
    if out.is_empty() {
        "root".to_string()
    } else {
        out
    }
}
pub(super) fn assert_dynamic_route_supported(route: &str) -> Result<(), String> {
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
pub(super) fn static_prefix(route: &str) -> String {
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
pub(super) fn ancestor_routes(route: &str) -> Vec<String> {
    if route == "/" {
        return vec!["/".to_string()];
    }
    let parts: Vec<&str> = route
        .trim_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
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
pub(super) fn dynamic_param_names(route: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_from_relative_path_table() {
        let cases: &[(FrameworkEntryKind, &str, Option<&str>)] = &[
            (FrameworkEntryKind::Page, "page.dsx", Some("/")),
            (FrameworkEntryKind::Page, "users/page.dsx", Some("/users")),
            (
                FrameworkEntryKind::Layout,
                "dashboard/layout.dsx",
                Some("/dashboard"),
            ),
            (FrameworkEntryKind::Layout, "layout.dsx", Some("/")),
            (
                FrameworkEntryKind::Api,
                "api/packages.dsx",
                Some("/api/packages"),
            ),
            (
                FrameworkEntryKind::Page,
                "blog/[slug]/page.dsx",
                Some("/blog/[slug]"),
            ),
            // RFD 24 §12: `.phpx` files are no longer framework entries.
            (FrameworkEntryKind::Page, "page.phpx", None),
            (FrameworkEntryKind::Layout, "dashboard/layout.phpx", None),
            (FrameworkEntryKind::Api, "api/packages.phpx", None),
        ];
        for (kind, rel, expected) in cases {
            assert_eq!(
                route_from_relative_path(*kind, rel).as_deref(),
                *expected,
                "route_from_relative_path({kind:?}, {rel:?})"
            );
        }
    }

    #[test]
    fn normalize_request_path_table() {
        let cases: &[(&str, &str)] = &[
            ("", "/"),
            ("/", "/"),
            ("blog", "/blog"),
            ("/blog/", "/blog"),
            ("/blog/?x=1", "/blog"),
            ("  /padded  ", "/padded"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_request_path(raw), *expected, "input {raw:?}");
        }
    }

    #[test]
    fn route_pattern_matches_table() {
        let cases: &[(&str, &str, Option<&[(&str, &str)]>)] = &[
            ("/", "/", Some(&[])),
            ("/", "/x", None),
            ("/blog", "/blog", Some(&[])),
            ("/blog", "/blog/x", None),
            ("/blog/[slug]", "/blog/hello", Some(&[("slug", "hello")])),
            ("/blog/[slug]", "/blog/hello/extra", None),
            // Catch-all segments are rejected (v1 supports one trailing param).
            ("/blog/[...slug]", "/blog/hello", None),
        ];
        for (pattern, path, expected) in cases {
            let got = route_pattern_matches(pattern, path);
            match expected {
                Some(pairs) => {
                    let want: BTreeMap<String, String> = pairs
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect();
                    assert_eq!(got.as_ref(), Some(&want), "{pattern:?} vs {path:?}");
                }
                None => assert_eq!(got, None, "{pattern:?} vs {path:?}"),
            }
        }
    }

    #[test]
    fn segment_helpers_table() {
        let param_cases: &[(&str, &[&str])] = &[
            ("/blog/[slug]", &["slug"]),
            ("/about", &[]),
            ("/[...rest]", &[]),
        ];
        for (route, expected) in param_cases {
            assert_eq!(dynamic_param_names(route), *expected, "params {route:?}");
        }
        let prefix_cases: &[(&str, &str)] = &[
            ("/blog/[slug]", "/blog/"),
            ("/[slug]", "/"),
            ("/a/b/[c]", "/a/b/"),
        ];
        for (route, expected) in prefix_cases {
            assert_eq!(static_prefix(route), *expected, "prefix {route:?}");
        }
        let ancestor_cases: &[(&str, &[&str])] = &[
            ("/", &["/"]),
            ("/blog", &["/blog", "/"]),
            ("/a/b/c", &["/a/b/c", "/a/b", "/a", "/"]),
        ];
        for (route, expected) in ancestor_cases {
            assert_eq!(ancestor_routes(route), *expected, "ancestors {route:?}");
        }
    }

    #[test]
    fn dynamic_routes_must_be_a_single_trailing_param() {
        assert!(assert_dynamic_route_supported("/blog/[slug]").is_ok());
        assert!(assert_dynamic_route_supported("/blog/[id]/comments").is_err());
        assert!(assert_dynamic_route_supported("/[a]/[b]").is_err());
    }

    #[test]
    fn slug_encodings_are_exact() {
        assert_eq!(route_css_slug("/"), "root");
        assert_eq!(route_css_slug("/a/b"), "a-s-b");
        assert_eq!(route_css_slug("/blog/[id]"), "blog-s--l-id-r-");
        assert_eq!(route_css_slug("/a.b"), "a-x2e-b");
        assert_eq!(ident_slug("/"), "root");
        assert_eq!(ident_slug("/a/b"), "a_s_b");
        assert_eq!(ident_slug("/a b"), "a_x20_b");
    }

    #[test]
    fn slugs_are_injective_over_the_route_charset() {
        // The old `_ => "-x-"`/`_x_` fallback collided for any two chars
        // outside the explicitly-mapped set (`/a.b` vs `/a b` both became
        // `a-x-b`). Sweep a charset covering every mapped char plus a
        // fallback sample and require distinct slugs for distinct routes.
        //
        // Domain note: the sweep builds *canonical* routes (one leading `/`,
        // no trailing `/`) — the only shape `route_from_relative_path` can
        // produce. `trim_matches('/')` deliberately makes `/a/` and `/a`
        // share a slug, and that equivalence is wanted.
        let charset: &[char] = &[
            '/', '_', '-', '[', ']', 'a', 'b', '9', '.', ' ', ':', '*', '~', '%', '!', '"',
        ];
        let routes: Vec<String> = charset
            .iter()
            .map(|c| format!("/a{c}b"))
            .chain(charset.iter().flat_map(|c| {
                charset
                    .iter()
                    .filter(move |d| *d != c)
                    .map(move |d| format!("/a{c}{d}b"))
            }))
            .collect();
        for slug_fn in [route_css_slug as fn(&str) -> String, ident_slug] {
            let mut seen: BTreeMap<String, &str> = BTreeMap::new();
            for route in &routes {
                let slug = slug_fn(route);
                if let Some(other) = seen.get(&slug) {
                    panic!("slug collision: {route:?} and {other:?} both map to {slug:?}");
                }
                seen.insert(slug, route);
            }
        }
    }

    #[test]
    fn cloudflare_redirects_true_does_not_emit_looping_splat() {
        let rules = cloudflare_redirects(true);
        assert!(!rules.contains("/* /:splat/"));
        assert!(cloudflare_redirects(false).contains("/*/ /:splat 301"));
    }
}
