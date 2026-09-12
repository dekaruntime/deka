//! Integration tests over the public `runtime_core::dist` route API —
//! these exercise the surface external callers (`runtime`, `cli`, `engine`)
//! actually use.

use runtime_core::dist::{
    FrameworkEntry, FrameworkEntryKind, FrameworkManifest, layout_chain, match_path,
};

fn entry(kind: FrameworkEntryKind, route: &str, file: &str) -> FrameworkEntry {
    FrameworkEntry {
        kind,
        route: route.into(),
        file: file.into(),
    }
}

#[test]
fn layout_chain_is_root_then_nested() {
    let entries = vec![
        entry(FrameworkEntryKind::Layout, "/", "app/layout.dsx"),
        entry(FrameworkEntryKind::Layout, "/blog", "app/blog/layout.dsx"),
        entry(FrameworkEntryKind::Page, "/blog", "app/blog/page.dsx"),
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
            entry(FrameworkEntryKind::Layout, "/", "app/layout.dsx"),
            entry(FrameworkEntryKind::Page, "/", "app/page.dsx"),
        ],
        not_found: Some(entry(FrameworkEntryKind::Page, "/", "app/not-found.dsx")),
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
        entries: vec![entry(
            FrameworkEntryKind::Page,
            "/blog/[slug]",
            "app/blog/[slug]/page.dsx",
        )],
        not_found: None,
    };
    let hit = match_path(&manifest, "/blog/hello");
    assert_eq!(hit.status, 200);
    assert_eq!(hit.params.get("slug").map(String::as_str), Some("hello"));
    let miss = match_path(&manifest, "/blog/hello/extra");
    assert_eq!(miss.status, 404);
}
