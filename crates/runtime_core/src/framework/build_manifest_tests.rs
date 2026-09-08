use super::*;
use crate::framework::manifest::{FrameworkEntry, FrameworkManifest};

fn page(template: &str, file: &str) -> FrameworkEntry {
    FrameworkEntry {
        kind: FrameworkEntryKind::Page,
        route: template.to_string(),
        file: file.to_string(),
    }
}

fn api(route: &str, file: &str) -> FrameworkEntry {
    FrameworkEntry {
        kind: FrameworkEntryKind::Api,
        route: route.to_string(),
        file: file.to_string(),
    }
}

fn params_descriptor(fields: &[(&str, &str)]) -> serde_json::Value {
    serde_json::json!({
        "node": "array",
        "elem": {
            "node": "struct",
            "fields": fields.iter().map(|(name, kind)| serde_json::json!({
                "name": name,
                "optional": false,
                "ty": { "node": "leaf", "kind": kind, "name": kind }
            })).collect::<Vec<_>>()
        }
    })
}

fn slot(id: &str, binding: &str, file: &str, descriptor: serde_json::Value) -> BuildPlanSlot {
    BuildPlanSlot {
        id: id.to_string(),
        binding: binding.to_string(),
        file: file.to_string(),
        span: serde_json::json!({}),
        descriptor,
        entry: "export default async function __deka_dev_x() {}".to_string(),
    }
}

fn plan(version: u32, slots: Vec<BuildPlanSlot>) -> BuildPlan {
    BuildPlan {
        version,
        prerender: None,
        slots,
    }
}

fn planned(file: &str, plan: BuildPlan) -> PlannedSource {
    PlannedSource {
        file: file.to_string(),
        plan,
    }
}

fn app(pages: &[FrameworkEntry]) -> FrameworkManifest {
    FrameworkManifest {
        root: "app".to_string(),
        entries: pages.to_vec(),
        not_found: None,
    }
}

fn plan_manifest(
    planned: &[PlannedSource],
    pages: &[FrameworkEntry],
    apis: &[FrameworkEntry],
) -> Result<BuildManifest, String> {
    BuildManifest::plan(Path::new("/project"), planned, None, &app(pages), apis)
}

#[test]
fn rejects_unsupported_plan_version() {
    let err = validate_plans(&[planned("app/x.ds", plan(3, vec![]))]).expect_err("v3 rejected");
    assert!(err.contains("unsupported dsc build plan version 3"), "{err}");
    assert!(err.contains("1, 2"), "{err}");
    let err = validate_plans(&[planned("app/x.ds", plan(0, vec![]))]).expect_err("v0 rejected");
    assert!(err.contains("unsupported"), "{err}");
}

#[test]
fn rejects_malformed_and_duplicate_slots() {
    let mut bad = slot("a", "x", "app/page.ds", serde_json::json!({}));
    bad.entry.clear();
    assert!(validate_plans(&[planned("app/page.ds", plan(2, vec![bad]))]).is_err());

    // Duplicate ids across different files' plans are still duplicates.
    let dup = vec![
        planned(
            "app/a.ds",
            plan(2, vec![slot("a", "x", "app/a.ds", serde_json::json!({}))]),
        ),
        planned(
            "app/b.ds",
            plan(2, vec![slot("a", "y", "app/b.ds", serde_json::json!({}))]),
        ),
    ];
    let err = validate_plans(&dup).expect_err("duplicate id rejected");
    assert!(err.contains("duplicate"), "{err}");
}

#[test]
fn static_and_api_routes_plan_without_instances() {
    let manifest = plan_manifest(
        &[],
        &[page("/", "app/page.dsx"), page("/about", "app/about/page.ds")],
        &[api("/api/hello", "api/hello.ds")],
    )
    .expect("plan builds");
    let table = manifest.render_route_table();
    assert!(table.contains("○ / "), "{table}");
    assert!(table.contains("○ /about"), "{table}");
    assert!(table.contains("λ /api/hello"), "{table}");
    assert!(!table.contains('●'), "{table}");
    assert!(!table.contains('ƒ'), "{table}");
}

#[test]
fn dynamic_route_without_disposition_fails() {
    let err = plan_manifest(&[], &[page("/posts/[slug]", "app/posts/page.ds")], &[])
        .expect_err("no disposition");
    assert!(
        err.contains("neither staticParams nor prerender = false"),
        "{err}"
    );
}

#[test]
fn dynamic_route_with_v1_plan_fails_with_upgrade_hint() {
    // A v1 plan carries no disposition, so a bracket route cannot be
    // classified — fail rather than guess.
    let err = plan_manifest(
        &[planned("app/posts/page.ds", plan(1, vec![]))],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect_err("v1 plan has no disposition");
    assert!(err.contains("neither staticParams nor prerender = false"), "{err}");
}

#[test]
fn prerender_false_marks_request_time() {
    let mut p = plan(2, vec![]);
    p.prerender = Some(false);
    let manifest = plan_manifest(
        &[planned("app/dashboard/page.ds", p)],
        &[page("/dashboard", "app/dashboard/page.ds")],
        &[],
    )
    .expect("plan builds");
    let table = manifest.render_route_table();
    assert!(table.contains("ƒ /dashboard"), "{table}");
    assert!(table.contains("prerender = false"), "{table}");
}

#[test]
fn static_params_on_static_page_fails() {
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/page.dsx",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    let err = plan_manifest(
        &[planned("app/page.dsx", p)],
        &[page("/", "app/page.dsx")],
        &[],
    )
    .expect_err("staticParams without bracket");
    assert!(err.contains("no [parameter]"), "{err}");
}

#[test]
fn descriptor_mismatches_fail_by_name() {
    // Non-struct element.
    let p = plan(
        2,
        vec![slot("s1", "staticParams", "app/posts/page.ds", serde_json::json!({"node":"array","elem":{"node":"leaf","kind":"string"}}))],
    );
    let err = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect_err("non-struct element");
    assert!(err.contains("must be structs"), "{err}");

    // Field name mismatch.
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("id", "string")]),
        )],
    );
    let err = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect_err("field mismatch");
    assert!(err.contains("do not match the route parameters"), "{err}");
    assert!(err.contains("slug"), "{err}");

    // Non-string field.
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "number")]),
        )],
    );
    let err = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect_err("non-string field");
    assert!(err.contains("must be a string"), "{err}");
}

#[test]
fn static_params_and_prerender_conflict() {
    let mut p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    p.prerender = Some(false);
    let err = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect_err("conflicting dispositions");
    assert!(err.contains("both staticParams"), "{err}");
}

#[test]
fn static_params_expand_into_instances() {
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    let mut manifest = plan_manifest(
        &[planned("/project/app/posts/page.ds", p)],
        &[page("/posts/[slug]", "/project/app/posts/page.ds")],
        &[],
    )
    .expect("plan builds");
    // Absolute vs project-relative file spellings must pair up.
    let values = BTreeMap::from([(
        "s1".to_string(),
        serde_json::json!([{"slug": "hello"}, {"slug": "world"}]),
    )]);
    manifest.expand_static_params(&values).expect("expands");
    let table = manifest.render_route_table();
    assert!(table.contains("● /posts/hello"), "{table}");
    assert!(table.contains("● /posts/world"), "{table}");
    assert!(table.contains("static: staticParams"), "{table}");
}

#[test]
fn instance_collisions_fail_naming_both_routes() {
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    let mut manifest = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[
            page("/posts/[slug]", "app/posts/page.ds"),
            page("/posts/hello", "app/posts/hello/page.ds"),
        ],
        &[],
    )
    .expect("plan builds");
    let values = BTreeMap::from([(
        "s1".to_string(),
        serde_json::json!([{"slug": "hello"}]),
    )]);
    let err = manifest.expand_static_params(&values).expect_err("collision");
    assert!(err.contains("/posts/hello"), "{err}");
    assert!(err.contains("collides"), "{err}");
}

#[test]
fn duplicate_slug_within_params_fails() {
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    let mut manifest = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect("plan builds");
    let values = BTreeMap::from([(
        "s1".to_string(),
        serde_json::json!([{"slug": "same"}, {"slug": "same"}]),
    )]);
    let err = manifest.expand_static_params(&values).expect_err("duplicate");
    assert!(err.contains("appears twice"), "{err}");
}

#[test]
fn unsafe_param_values_fail() {
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    let mut manifest = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect("plan builds");
    // "." and ".." are the dangerous cases: the filesystem normalizes them,
    // so they collide with sibling routes or escape dist/client entirely.
    for raw in ["a/b", "a\\b", "", ".", ".."] {
        let values =
            BTreeMap::from([("s1".to_string(), serde_json::json!([{"slug": raw}]))]);
        let err = manifest
            .expand_static_params(&values)
            .expect_err("value `{raw}` must be rejected");
        assert!(err.contains("safe path segment"), "{err}");
    }
}

#[test]
fn missing_materialized_value_fails_before_render() {
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    let mut manifest = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect("plan builds");
    let err = manifest
        .expand_static_params(&BTreeMap::new())
        .expect_err("no value");
    assert!(err.contains("was not materialized"), "{err}");
}

#[test]
fn duplicate_page_templates_fail() {
    let err = plan_manifest(
        &[],
        &[
            page("/about", "app/about/page.ds"),
            page("/about", "app/about/page.dsx"),
        ],
        &[],
    )
    .expect_err("same template twice");
    assert!(err.contains("two app pages"), "{err}");
}

#[test]
fn manifest_is_deterministic_across_repeated_plans() {
    let pages = vec![
        page("/", "app/page.dsx"),
        page("/posts/[slug]", "app/posts/page.ds"),
        page("/about", "app/about/page.ds"),
    ];
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    let mut manifests = Vec::new();
    for _ in 0..3 {
        let mut m = plan_manifest(
            &[planned("app/posts/page.ds", p.clone())],
            &pages,
            &[api("/api/x", "api/x.ds")],
        )
        .expect("plan builds");
        m.expand_static_params(&BTreeMap::from([(
            "s1".to_string(),
            serde_json::json!([{"slug": "hello"}]),
        )]))
        .expect("expands");
        manifests.push(m.canonical_json().expect("json"));
    }
    assert!(
        manifests.windows(2).all(|pair| pair[0] == pair[1]),
        "identical inputs must produce identical manifest bytes"
    );
}

#[test]
fn record_artifacts_hashes_every_file_sorted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let staged = temp.path();
    std::fs::create_dir_all(staged.join("client/posts")).expect("mkdir");
    std::fs::write(staged.join("client/index.html"), b"<html/>").expect("write");
    std::fs::write(staged.join("client/posts/index.html"), b"post").expect("write");
    std::fs::write(staged.join("_redirects"), b"/* / 301").expect("write");

    let mut manifest = plan_manifest(&[], &[page("/", "app/page.dsx")], &[])
        .expect("plan builds");
    manifest.record_artifacts(staged).expect("records");
    assert_eq!(manifest.artifacts.len(), 3);
    let paths: Vec<&str> = manifest.artifacts.iter().map(|a| a.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "artifacts are sorted by path");
    let html = std::fs::read(staged.join("client/index.html")).expect("read");
    assert_eq!(
        manifest.artifacts.iter().find(|a| a.path == "client/index.html").expect("row").digest,
        sha256_hex(&html)
    );
    // Directory entries and the manifest's own cache file never live
    // under staged dist, but an empty tree must record no artifacts.
    let mut empty = plan_manifest(&[], &[page("/", "app/page.dsx")], &[])
        .expect("plan builds");
    empty.record_artifacts(&staged.join("absent")).expect("records");
    assert!(empty.artifacts.is_empty());
}

#[test]
fn project_relative_normalizes_spellings() {
    let root = Path::new("/project");
    assert_eq!(project_relative(root, "/project/app/page.ds"), "app/page.ds");
    assert_eq!(project_relative(root, "app/page.ds"), "app/page.ds");
    assert_eq!(project_relative(root, "./app/page.ds"), "app/page.ds");
    assert_eq!(
        project_relative(root, "\\project\\app\\page.ds"),
        "/project/app/page.ds"
    );
}

#[test]
fn slot_records_carry_identity() {
    let p = plan(
        2,
        vec![slot(
            "s1",
            "staticParams",
            "app/posts/page.ds",
            params_descriptor(&[("slug", "string")]),
        )],
    );
    let descriptor = p.slots[0].descriptor.to_string();
    let manifest = plan_manifest(
        &[planned("app/posts/page.ds", p)],
        &[page("/posts/[slug]", "app/posts/page.ds")],
        &[],
    )
    .expect("plan builds");
    assert_eq!(manifest.slots.len(), 1);
    let slot = &manifest.slots[0];
    assert_eq!(slot.value_module, "deka:dev/s1");
    let expected = sha256_hex(descriptor.as_bytes());
    assert_eq!(slot.descriptor_digest, expected);
    assert_eq!(manifest.compiler.plan_version, 2);
}
