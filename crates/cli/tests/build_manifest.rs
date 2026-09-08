// deka#719: the validated build manifest drives static rendering and the
// printed route table, and `dist/` publication is transactional (staged
// tree, atomic swap, interrupted-publish recovery).
//
// Real dsc required (the suite's other build tests assume it too). Released
// dsc is 0.6.x and emits build plan version 1 (no `prerender` disposition),
// which gates the prerender_request_time variants.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Scaffolds a fresh web project into `dir` via `deka init`, exactly the way
/// a human would, and asserts the scaffold succeeded.
fn init_project(dir: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(dir)
        .output()
        .expect("run deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        dir.join("app").join("page.dsx").is_file(),
        "deka init should scaffold app/page.dsx"
    );
}

fn run_build(dir: &Path) -> (bool, String) {
    run_build_with_env(dir, &[])
}

fn run_build_with_env(dir: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let mut command = Command::new(cli_bin());
    command.arg("build").current_dir(dir);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("run deka build");
    // stdio emits to stderr (repo convention); table and diagnostics share it.
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// Route-table rows from build output (stdio emits to stderr by convention),
/// normalized to `glyph path` pairs (the detail column is dropped; route
/// paths do not contain whitespace).
fn route_table_rows(output: &str) -> BTreeSet<String> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let is_row = trimmed.starts_with("○ ")
                || trimmed.starts_with("● ")
                || trimmed.starts_with('ƒ')
                || trimmed.starts_with('λ');
            if !is_row {
                return None;
            }
            let mut tokens = trimmed.split_whitespace();
            let glyph = tokens.next()?;
            let path = tokens.next()?;
            Some(format!("{glyph} {path}"))
        })
        .collect()
}

fn manifest_path(project: &Path) -> PathBuf {
    project
        .join(".cache")
        .join("dekascript")
        .join("build-manifest.json")
}

/// Walk `dir` into a relative-path -> bytes map for tree comparison.
fn tree_snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).expect("read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .expect("rel")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, fs::read(&path).expect("read"));
            }
        }
    }
    out
}

fn real_dsc() -> PathBuf {
    if let Ok(path) = std::env::var("DEKA_DSC") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let output = Command::new("which")
        .arg("dsc")
        .output()
        .expect("run which dsc");
    assert!(
        output.status.success(),
        "tests require dsc (set DEKA_DSC or put dsc on PATH)"
    );
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

/// The build plan version the installed dsc emits; decides which
/// prerender_request_time variant runs.
fn dsc_plan_version() -> u32 {
    let dir = tempfile::tempdir().expect("create probe dir");
    let file = dir.path().join("probe.ds");
    fs::write(&file, "export const prerender = false\n").expect("write probe");
    let output = Command::new(real_dsc())
        .arg("plan")
        .arg(&file)
        .current_dir(dir.path())
        .output()
        .expect("run dsc plan probe");
    assert!(
        output.status.success(),
        "dsc plan probe failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse probe plan");
    plan.get("version")
        .and_then(|v| v.as_u64())
        .expect("probe plan has a version") as u32
}

#[test]
fn static_params_end_to_end() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let slug_dir = project.path().join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    fs::write(
        slug_dir.join("page.dsx"),
        "interface PageProps { slug: string }\nstruct PostParam { slug: string }\nexport const staticParams: Array<PostParam> = build {\n    return Ok([PostParam{slug:\"hello\"}, PostParam{slug:\"world\"}])\n}\nexport fn Page(props: PageProps) {\n    return <article><h1>{props.slug}</h1></article>;\n}\n",
    )
    .expect("write slug page");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with staticParams: {combined}"
    );

    for slug in ["hello", "world"] {
        let html_path = project
            .path()
            .join("dist")
            .join("client")
            .join("posts")
            .join(slug)
            .join("index.html");
        let html = fs::read_to_string(&html_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", html_path.display()));
        assert!(
            html.contains(&format!(">{slug}<")),
            "prerendered HTML for /posts/{slug} should contain the slug: {html}"
        );
    }

    let rows = route_table_rows(&combined);
    assert!(rows.iter().any(|r| r.contains("● /posts/hello")), "table should show ● /posts/hello: {combined}");
    assert!(rows.iter().any(|r| r.contains("● /posts/world")), "table should show ● /posts/world: {combined}");
    assert!(rows.iter().any(|r| r.contains("○ /")), "table should show ○ /: {combined}");

    // The manifest records one ● instance row per materialized value.
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(manifest_path(project.path())).expect("read build manifest"),
    )
    .expect("parse build manifest");
    let instances: Vec<&str> = manifest["routes"]
        .as_array()
        .expect("routes array")
        .iter()
        .filter_map(|route| route["instance"].as_str())
        .collect();
    assert!(
        instances.contains(&"/posts/hello") && instances.contains(&"/posts/world"),
        "manifest instances should include both posts: {manifest}"
    );
}

#[test]
fn dynamic_route_without_disposition_fails() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let id_dir = project.path().join("app").join("[id]");
    fs::create_dir_all(&id_dir).expect("mkdir [id]");
    fs::write(
        id_dir.join("page.dsx"),
        "interface PageProps { id: string }\nexport fn Page(props: PageProps) {\n    return <article>{props.id}</article>;\n}\n",
    )
    .expect("write id page");

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must fail when a dynamic route declares no disposition"
    );
    assert!(
        combined.contains("neither staticParams nor prerender = false"),
        "failure should name the missing disposition: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "a failed build must not publish dist/: {combined}"
    );
}

#[test]
fn route_collision_fails() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let slug_dir = project.path().join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    fs::write(
        slug_dir.join("page.dsx"),
        "interface PageProps { slug: string }\nstruct PostParam { slug: string }\nexport const staticParams: Array<PostParam> = build {\n    return Ok([PostParam{slug:\"hello\"}])\n}\nexport fn Page(props: PageProps) {\n    return <article>{props.slug}</article>;\n}\n",
    )
    .expect("write slug page");
    let concrete_dir = project.path().join("app").join("posts").join("hello");
    fs::create_dir_all(&concrete_dir).expect("mkdir posts/hello");
    fs::write(
        concrete_dir.join("page.dsx"),
        "export fn Page() {\n    return <article>concrete hello</article>;\n}\n",
    )
    .expect("write concrete page");

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must fail when a staticParams instance collides with a concrete page"
    );
    assert!(
        combined.contains("route collision") && combined.contains("/posts/hello"),
        "failure should name the collision: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "a failed build must not publish dist/: {combined}"
    );
}

#[test]
fn failed_render_rolls_back() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    // First build succeeds; seed a marker into the published dist.
    let (success, combined) = run_build(project.path());
    assert!(success, "first build should succeed: {combined}");
    let marker = project.path().join("dist").join("marker.txt");
    fs::write(&marker, "v1-dist").expect("write marker");
    let before = tree_snapshot(project.path().join("dist").as_path());

    // Now break the page: a top-level throw fails the static render.
    fs::write(
        project.path().join("app").join("page.dsx"),
        "const doomed: string = unsafe { throw new Error(\"boom\") }\nexport fn Page() {\n    return <h1>hi {doomed}</h1>;\n}\n",
    )
    .expect("write throwing page");

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must fail when the page render throws: {combined}"
    );

    let after = tree_snapshot(project.path().join("dist").as_path());
    assert_eq!(
        before, after,
        "a failed build must leave the previous dist byte-identical: {combined}"
    );
    assert_eq!(
        fs::read_to_string(&marker).expect("read marker"),
        "v1-dist",
        "pre-existing dist content must survive a failed build"
    );
}

#[test]
fn deterministic_repeated_build() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let slug_dir = project.path().join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    fs::write(
        slug_dir.join("page.dsx"),
        "interface PageProps { slug: string }\nstruct PostParam { slug: string }\nexport const staticParams: Array<PostParam> = build {\n    return Ok([PostParam{slug:\"hello\"}, PostParam{slug:\"world\"}])\n}\nexport fn Page(props: PageProps) {\n    return <article>{props.slug}</article>;\n}\n",
    )
    .expect("write slug page");

    let (success, combined_a) = run_build(project.path());
    assert!(success, "first build should succeed: {combined_a}");
    let manifest_a = fs::read(manifest_path(project.path())).expect("read manifest");
    let tree_a = tree_snapshot(project.path().join("dist").as_path());

    let (success, combined_b) = run_build(project.path());
    assert!(success, "second build should succeed: {combined_b}");
    let manifest_b = fs::read(manifest_path(project.path())).expect("read manifest");
    let tree_b = tree_snapshot(project.path().join("dist").as_path());

    assert_eq!(
        route_table_rows(&combined_a),
        route_table_rows(&combined_b),
        "route tables must be identical across rebuilds"
    );
    assert_eq!(
        manifest_a, manifest_b,
        "build-manifest.json must be byte-identical across rebuilds"
    );
    assert_eq!(tree_a, tree_b, "dist/ trees must be byte-identical across rebuilds");
    assert!(
        !project.path().join(".deka-dist-stage").exists(),
        "staging dir must not leak into the project"
    );
}

#[test]
fn table_matches_manifest() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let slug_dir = project.path().join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    fs::write(
        slug_dir.join("page.dsx"),
        "interface PageProps { slug: string }\nstruct PostParam { slug: string }\nexport const staticParams: Array<PostParam> = build {\n    return Ok([PostParam{slug:\"hello\"}])\n}\nexport fn Page(props: PageProps) {\n    return <article>{props.slug}</article>;\n}\n",
    )
    .expect("write slug page");

    let (success, combined) = run_build(project.path());
    assert!(success, "build should succeed: {combined}");

    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(manifest_path(project.path())).expect("read build manifest"),
    )
    .expect("parse build manifest");
    let mut expected: BTreeSet<String> = BTreeSet::new();
    for route in manifest["routes"].as_array().expect("routes array") {
        let glyph = match route["mode"].as_str().expect("route mode") {
            "static" => "○",
            "static_params" => "●",
            "request_time" => "ƒ",
            "api" => "λ",
            other => panic!("unknown route mode {other}"),
        };
        let path = route["instance"]
            .as_str()
            .or_else(|| route["template"].as_str())
            .expect("route path");
        expected.insert(format!("{glyph} {path}"));
    }

    let rows = route_table_rows(&combined);
    assert_eq!(
        rows, expected,
        "route table rows must match the manifest routes exactly"
    );
}

#[test]
fn prerender_request_time() {
    let version = dsc_plan_version();
    eprintln!("prerender_request_time: dsc emits plan version {version}");

    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    // A bracket route: with no staticParams slot, only a v2 plan's
    // `prerender = false` disposition can classify it.
    let user_dir = project.path().join("app").join("user").join("[id]");
    fs::create_dir_all(&user_dir).expect("mkdir user/[id]");
    fs::write(
        user_dir.join("page.dsx"),
        "interface PageProps { id: string }\nexport const prerender = false\nexport fn Page(props: PageProps) {\n    return <article>{props.id}</article>;\n}\n",
    )
    .expect("write prerender=false page");

    let (success, combined) = run_build(project.path());
    if version >= 2 {
        assert!(
            success,
            "dsc v{version} reports disposition; build should succeed: {combined}"
        );
        let rows = route_table_rows(&combined);
        assert!(
            rows.iter().any(|r| r.contains('ƒ') && r.contains("/user/[id]")),
            "table should show ƒ /user/[id]: {combined}"
        );
        assert!(
            !project.path().join("dist").join("client").join("user").exists(),
            "request-time routes must not publish static HTML"
        );
    } else {
        assert!(
            !success,
            "dsc v{version} reports no disposition; the build must fail closed"
        );
        assert!(
            combined.contains("neither staticParams nor prerender = false"),
            "failure should name the missing disposition: {combined}"
        );
        assert!(
            combined.contains("upgrade dsc"),
            "failure should point at upgrading dsc: {combined}"
        );
        assert!(
            !project.path().join("dist").exists(),
            "a failed build must not publish dist/: {combined}"
        );
    }
}

#[test]
fn dynamic_only_app_publishes_no_index() {
    let version = dsc_plan_version();
    eprintln!("dynamic_only_app_publishes_no_index: dsc emits plan version {version}");

    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    // Every route is request-time: the build must succeed, publish no static
    // HTML, and not fabricate a root render.
    let root_page = project.path().join("app").join("page.dsx");
    fs::write(
        &root_page,
        "export const prerender = false\nexport fn Page() {\n    return <main>dynamic</main>;\n}\n",
    )
    .expect("write dynamic root page");

    let (success, combined) = run_build(project.path());
    if version >= 2 {
        assert!(success, "a dynamic-only app must build: {combined}");
        assert!(
            !project
                .path()
                .join("dist")
                .join("client")
                .join("index.html")
                .exists(),
            "an app whose routes are all request-time must not fabricate a root render: {combined}"
        );
        let rows = route_table_rows(&combined);
        assert!(
            rows.iter().any(|r| r == "ƒ /"),
            "table should show the request-time root route: {combined}"
        );
    } else {
        // v1 plans carry no disposition: the root route plans static and
        // renders as before. Documents that skipping is gated on manifest
        // disposition, not a blanket behavior change.
        assert!(
            success,
            "v{version} projects keep their legacy static-root behavior: {combined}"
        );
        assert!(
            project
                .path()
                .join("dist")
                .join("client")
                .join("index.html")
                .exists(),
            "v{version} renders the root route statically: {combined}"
        );
    }
}

#[test]
fn stale_plan_rejected() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    // Fake dsc: stale plan output for `plan`, everything else delegated to
    // the real binary. Exercises the pre-execution validation boundary.
    let real = real_dsc();
    let fake = project.path().join("fake-dsc.sh");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"plan\" ]; then\n  echo '{{\"version\": 99, \"slots\": []}}'\n  exit 0\nfi\nexec '{}' \"$@\"\n",
            real.display()
        ),
    )
    .expect("write fake dsc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fake).expect("stat fake").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake, perms).expect("chmod fake");
    }

    let (success, combined) = run_build_with_env(
        project.path(),
        &[("DEKA_DSC", fake.to_str().expect("fake path utf8"))],
    );
    assert!(
        !success,
        "deka build must reject an unsupported dsc build plan version"
    );
    assert!(
        combined.contains("unsupported dsc build plan version 99"),
        "failure should name the stale plan version: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "a rejected plan must not publish dist/: {combined}"
    );
}
