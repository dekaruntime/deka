use super::*;
use std::collections::BTreeMap;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(&path, body).expect("write");
}

#[test]
fn relative_path_computes_forward_slash_parents() {
    let root = Path::new("/project/dist");
    let dist_server = root.join("server");
    let importer = root.join("server").join("app").join("posts").join("[slug]");
    let target = dist_server.join(VALUES_DIR).join("abc.js");
    assert_eq!(relative_path(&importer, &target), "../../../.values/abc.js");
    assert_eq!(relative_path(&dist_server, &target), ".values/abc.js");
}

#[test]
fn relative_specifier_computes_server_root_relative_paths() {
    assert_eq!(
        relative_specifier(Path::new("app/posts/[slug]"), Path::new("app/page.js")),
        "../../page.js"
    );
    assert_eq!(
        relative_specifier(Path::new(""), Path::new("app/page.js")),
        "./app/page.js"
    );
    assert_eq!(
        relative_specifier(Path::new("app"), Path::new(".values/abc.js")),
        "../.values/abc.js"
    );
}

#[test]
fn reroot_target_maps_entries_and_project_sources() {
    let root = Path::new("/project");
    let entries = root.join(".deka-dist-stage").join("entries");
    assert_eq!(
        reroot_target(root, &entries, &entries.join("serve-entry.dsx")),
        Some("serve-entry.js".to_string())
    );
    assert_eq!(
        reroot_target(root, &entries, &root.join("app/posts/[slug]/page.dsx")),
        Some("app/posts/[slug]/page.js".to_string())
    );
    assert_eq!(
        reroot_target(root, &entries, Path::new("/elsewhere/x.dsx")),
        None
    );
}

#[test]
fn rewrite_module_specifiers_reroots_relative_dev_and_ui_specs() {
    let root = Path::new("/project");
    let entries = root.join(".deka-dist-stage").join("entries");
    let page_source = root.join("app/page.dsx");
    let mut targets: BTreeMap<PathBuf, String> = BTreeMap::new();
    targets.insert(page_source.clone(), "app/page.js".to_string());
    targets.insert(
        entries.join("serve-entry.dsx"),
        "serve-entry.js".to_string(),
    );

    let entry_js = concat!(
        "import { Page as Page_ } from \"../../app/page.dsx\";\n",
        "import { Suspense } from \"ui/suspense\";\n",
        "import { hydrate as h } from \"deka:dev/abc123\";\n",
        "import { x } from \"@deka/encoding/json\";\n",
    );
    let mut ui = BTreeSet::new();
    let rewritten = rewrite_module_specifiers(
        entry_js,
        &entries.join("serve-entry.dsx"),
        &targets,
        root,
        "serve-entry.js",
        &mut ui,
    )
    .unwrap();
    assert!(rewritten.contains("\"./app/page.js\""), "{rewritten}");
    assert!(rewritten.contains("\"./.ui/suspense.js\""), "{rewritten}");
    assert!(rewritten.contains("\"./.values/abc123.js\""), "{rewritten}");
    // Bare package specifiers stay for the loader's ds_modules resolution.
    assert!(
        rewritten.contains("\"@deka/encoding/json\""),
        "{rewritten}"
    );
    assert!(!rewritten.contains("deka:dev/"), "{rewritten}");
    assert!(ui.contains("suspense.js"), "{ui:?}");
}

#[test]
fn rewrite_module_specifiers_fails_on_dangling_relative_import() {
    let root = Path::new("/project");
    let entries = root.join("stage").join("entries");
    let mut targets: BTreeMap<PathBuf, String> = BTreeMap::new();
    targets.insert(
        entries.join("serve-entry.dsx"),
        "serve-entry.js".to_string(),
    );
    let err = rewrite_module_specifiers(
        "import { x } from \"./missing.js\";\n",
        &entries.join("serve-entry.dsx"),
        &targets,
        root,
        "serve-entry.js",
        &mut BTreeSet::new(),
    )
    .unwrap_err();
    assert!(err.contains("dangling specifier"), "{err}");
}

#[test]
fn copy_pulls_only_referenced_ids() {
    let project = tempfile::tempdir().unwrap();
    let cache = project.path().join("cache").join("build-values");
    let dist_server = project.path().join("dist").join("server");
    write(&cache, "abc.js", "export const value = 1;\n");
    write(&cache, "stale.js", "export const value = 2;\n");
    let ids: BTreeSet<String> = ["abc".to_string()].into_iter().collect();
    copy_build_value_modules(&cache, &dist_server, &ids).unwrap();
    assert!(dist_server.join(VALUES_DIR).join("abc.js").is_file());
    assert!(!dist_server.join(VALUES_DIR).join("stale.js").exists());
}

#[test]
fn rewrite_swaps_known_ids_and_keeps_dist_self_contained() {
    let project = tempfile::tempdir().unwrap();
    let dist = project.path().join("dist");
    let dist_server = dist.join("server");
    write(
        &dist_server.join("app/posts/[slug]"),
        "page.js",
        "import { hydrate as b } from \"deka:dev/abc123\";\nconst page = 1;\n",
    );
    let ids: BTreeSet<String> = ["abc123".to_string()].into_iter().collect();
    rewrite_build_value_specifiers(&dist, &dist_server, &ids).unwrap();
    let rewritten =
        fs::read_to_string(dist_server.join("app/posts/[slug]/page.js")).unwrap();
    assert!(
        rewritten.contains("\"../../../.values/abc123.js\""),
        "{rewritten}"
    );
    assert!(!rewritten.contains("deka:dev/"), "{rewritten}");
}

#[test]
fn rewrite_fails_loudly_on_unmaterialized_specifier() {
    let project = tempfile::tempdir().unwrap();
    let dist = project.path().join("dist");
    let dist_server = dist.join("server");
    write(
        &dist_server,
        "page.js",
        "import { hydrate as b } from \"deka:dev/unknown\";\n",
    );
    let ids: BTreeSet<String> = ["abc123".to_string()].into_iter().collect();
    let err = rewrite_build_value_specifiers(&dist, &dist_server, &ids)
        .expect_err("an unknown deka:dev specifier must fail the build");
    assert!(err.contains("not self-contained"), "{err}");
}

#[test]
fn ui_rewrite_vendors_embedded_modules_and_updates_specifiers() {
    let project = tempfile::tempdir().unwrap();
    let dist_server = project.path().join("dist").join("server");
    write(
        &dist_server.join("app"),
        "page.js",
        "import { jsx } from \"ui/jsx\";\nexport default function Page() {}\n",
    );
    rewrite_ui_specifiers(&dist_server).unwrap();
    let rewritten = fs::read_to_string(dist_server.join("app/page.js")).unwrap();
    assert!(rewritten.contains("\"../.ui/jsx.js\""), "{rewritten}");
    assert!(!rewritten.contains("\"ui/jsx\""), "{rewritten}");
    // jsx's relative siblings are vendored too, so nothing dangles.
    let ui_dir = dist_server.join(UI_DIR);
    assert!(ui_dir.join("jsx.js").is_file());
    let jsx_source = fs::read_to_string(ui_dir.join("jsx.js")).unwrap();
    for spec in runtime_core::ds_imports::paths(&jsx_source) {
        let Some(rel) = spec.strip_prefix("./") else { continue };
        assert!(ui_dir.join(rel).is_file(), "{spec} not vendored");
    }
}

#[test]
fn server_jail_rejects_escaping_and_dangling_relative_specifiers() {
    let project = tempfile::tempdir().unwrap();
    let dist_server = project.path().join("dist").join("server");
    write(
        &dist_server.join("app"),
        "page.js",
        "import { x } from \"../../client/secret.js\";\n",
    );
    let err = assert_server_jail(&dist_server).unwrap_err();
    assert!(err.contains("outside dist/server"), "{err}");

    let project = tempfile::tempdir().unwrap();
    let dist_server = project.path().join("dist").join("server");
    write(&dist_server, "entry.js", "import { x } from \"./gone.js\";\n");
    let err = assert_server_jail(&dist_server).unwrap_err();
    assert!(err.contains("does not resolve"), "{err}");

    let project = tempfile::tempdir().unwrap();
    let dist_server = project.path().join("dist").join("server");
    write(&dist_server, "a.js", "export const a = 1;\n");
    write(&dist_server, "b.js", "import { a } from \"./a.js\";\n");
    assert_server_jail(&dist_server).unwrap();
}

#[test]
fn named_imports_reads_exported_side_of_as_aliases() {
    // `Page as Page_root` requires the module's export `Page`; the local
    // alias must not leak into the linkage check.
    let js = "import { Page as Page_root, Layout } from \"./app/page.js\";\nimport \"./side.js\";\n";
    let imports = named_imports(js);
    assert_eq!(imports.len(), 1, "side-effect imports carry no names: {imports:?}");
    let (spec, names) = &imports[0];
    assert_eq!(spec, "./app/page.js");
    assert_eq!(names, &vec!["Page".to_string(), "Layout".to_string()]);
}

#[test]
fn has_export_covers_declaration_shapes_and_brace_lists() {
    let js = concat!(
        "export function Page() {}\n",
        "export const ready = 1;\n",
        "export class Thing {}\n",
        "export { User, make as buildUser };\n",
    );
    for name in ["Page", "ready", "Thing", "User", "buildUser"] {
        assert!(has_export(js, name), "expected export {name}");
    }
    assert!(!has_export(js, "make"), "the left side of `as` is not exported");
    assert!(!has_export(js, "Page_root"));
    assert!(has_export("export default function() {}", "default"));
}

#[test]
fn entry_linkage_fails_when_the_winning_variant_lacks_an_export() {
    // The defer-first merge variant of a page module keeps only the deferred
    // component; the serve entry still imports Page — that must fail loudly.
    let project = tempfile::tempdir().unwrap();
    let dist_server = project.path().join("dist").join("server");
    write(&dist_server.join("app"), "page.js", "export function Badge() {}\n");
    write(
        &dist_server,
        "serve-entry.js",
        "import { Page as Page_root } from \"./app/page.js\";\n",
    );
    let emitted = EmittedEntries {
        serve: Some("server/serve-entry.js".to_string()),
        api: None,
        defer: None,
    };
    let err = assert_entry_linkage(&dist_server, &emitted).unwrap_err();
    assert!(err.contains("does not export"), "{err}");

    write(
        &dist_server.join("app"),
        "page.js",
        "export function Badge() {}\nexport function Page() {}\n",
    );
    assert_entry_linkage(&dist_server, &emitted).unwrap();
}
