//! Effect test for deka#622 finding A: CSS import-follow must resolve
//! extensionless relative specifiers the same way the rest of the runtime does
//! (`ds_source_candidates`).
//!
//! Existing coverage does not catch this:
//! - `css_plan_hoists_shared_classes` is handed pre-built `RouteStyle` structs.
//! - `collect_import_paths_reads_multiline_from` uses `from "./Card.dsx"`,
//!   matching the walker's extension filter rather than the resolver.
//!
//! This test asserts the emitted `route-*.css`, not the walker's intermediate
//! output. It must fail on a walker that drops `import Card from "./Card"`.

use std::fs;

#[test]
fn extensionless_component_import_emits_utility_class_in_route_css() {
    let tmp = tempfile::tempdir().expect("temp project");
    let root = tmp.path();
    fs::create_dir_all(root.join("app")).expect("mkdir app");
    // The source-project predicate is deka.json + app/page.ds(x) (the root
    // index.html is a build output, never a source requirement — deka#762).
    fs::write(root.join("deka.json"), "{}\n").expect("write deka.json");
    fs::write(
        root.join("index.html"),
        "<!doctype html><html><head><!--deka-head--></head><body><div id=\"app\"><!--deka-app--></div><!--deka-scripts--></body></html>\n",
    )
    .expect("write index.html");
    // The page itself uses `p-4` so a route stylesheet is still emitted if the
    // walker never follows `./Card`. The class under test lives only on Card.
    fs::write(
        root.join("app/page.dsx"),
        "import Card from \"./Card\"\nexport fn Page() {\n    return <main class=\"p-4\"><Card /></main>;\n}\n",
    )
    .expect("write page.dsx");
    fs::write(
        root.join("app/Card.dsx"),
        "export fn Card() {\n    return <article class=\"text-lg\">card</article>;\n}\n",
    )
    .expect("write Card.dsx");

    runtime::write_route_css_assets_for_project(root).expect("write route css");

    // #604 content-addresses stylesheets as `route-root.<hash>.css`.
    let css_dir = root
        .join(".cache")
        .join("dekascript")
        .join("assets")
        .join("css");
    let css = read_hashed_css(&css_dir, "route-root");
    assert!(
        css.contains(".text-lg"),
        "utility class from extensionless `import Card from \"./Card\"` (Card.dsx) must land in route CSS: {css}"
    );
}

fn read_hashed_css(dir: &std::path::Path, stem: &str) -> String {
    let prefix = format!("{stem}.");
    let path = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("css dir must exist ({err}); looked at {}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".css"))
        })
        .unwrap_or_else(|| {
            panic!(
                "{stem}.<hash>.css must be emitted; looked at {}",
                dir.display()
            )
        });
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}
