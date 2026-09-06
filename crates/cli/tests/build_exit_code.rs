// Regression test for deka#5: `deka build` must fail closed (non-zero exit,
// no dist/ output) when the web project's source under app/ does not
// compile -- matching what `deka check` already reports for that file.
//
// `deka build` prefers default dsc emit (`dsc --outdir`), falling back to
// per-tree `dsc transpile <dir> --out`, then copies host static files.
// Raw app/ .ds is not the server product.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Scaffolds a fresh web project into `dir` via `deka init`, exactly the way
/// a human would (per the issue's reproduction), and asserts the scaffold
/// succeeded.
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
        dir.join("deka.json").is_file(),
        "deka init should scaffold deka.json"
    );
    assert!(
        dir.join("deka.lock").is_file(),
        "deka init should scaffold deka.lock"
    );
    assert!(
        dir.join("app").join("page.dsx").is_file(),
        "deka init should scaffold app/page.dsx"
    );
    assert!(
        dir.join("index.html").is_file(),
        "deka init should scaffold index.html"
    );
}

fn run_build(dir: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(dir)
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// Find `<stem>.<10 lowercase hex>.<ext>` in `dir` (content-hashed assets).
fn find_hashed_asset(dir: &Path, stem: &str, ext: &str) -> Option<std::path::PathBuf> {
    let prefix = format!("{stem}.");
    let suffix = format!(".{ext}");
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) || !name.ends_with(&suffix) {
            return None;
        }
        let hash = &name[prefix.len()..name.len() - suffix.len()];
        let is_hash = hash.len() == 10
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        is_hash.then_some(entry.path())
    })
}

/// File name of the hashed asset, panicking with context when missing.
fn hashed_asset_name_in(dir: &Path, stem: &str, ext: &str) -> String {
    find_hashed_asset(dir, stem, ext)
        .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| panic!("{stem}.<hash>.{ext} not found in {}", dir.display()))
}

#[test]
fn build_exits_nonzero_on_invalid_source() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    // Same invalid snippet as the issue's reproduction: unterminated
    // function body, which `deka check` correctly rejects (exit 1).
    fs::write(
        project.path().join("app").join("page.dsx"),
        "export function f(): int { return\n",
    )
    .expect("write invalid app/page.dsx");

    // Sanity: `deka check` rejects this file on its own, confirming the
    // fixture is genuinely invalid and not an environment quirk.
    let check = Command::new(cli_bin())
        .args(["check", "app/page.dsx"])
        .current_dir(project.path())
        .output()
        .expect("run deka check");
    assert!(
        !check.status.success(),
        "fixture should be rejected by `deka check`; test fixture is not actually invalid"
    );

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must exit non-zero on invalid source under app/, got success. output: {combined}"
    );
    assert!(
        combined.contains("page.dsx"),
        "build failure diagnostic should name the offending file: {combined}"
    );

    // Fail-closed: no dist/ output should be produced when validation fails.
    assert!(
        !project.path().join("dist").exists(),
        "deka build should not write dist/ output when source fails to compile"
    );
}

#[test]
fn build_exits_zero_on_valid_source() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should exit 0 for the unmodified `deka init` scaffold: {combined}"
    );

    let index = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read dist/client/index.html");
    assert!(
        index.contains("Deka App"),
        "dist HTML should be filled, got: {index}"
    );
    assert!(
        !index.contains("<!--deka-app-->"),
        "dist HTML should not leave the app hole empty: {index}"
    );
    assert!(
        !index.contains("<script"),
        "a page with no client:* must not emit a script tag: {index}"
    );
    let dist_app = project.path().join("dist").join("app");
    assert!(
        dist_app.join("page.js").is_file()
            && dist_app.join("layout.js").is_file()
            && dist_app.join("not-found.js").is_file(),
        "successful build should emit app/ as .js via dsc into dist/app: {combined}"
    );
    assert!(
        !dist_app.join("page.dsx").exists()
            && !dist_app.join("layout.dsx").exists()
            && !dist_app.join("not-found.dsx").exists()
            && !project
                .path()
                .join("dist")
                .join("server")
                .join("app")
                .join("page.dsx")
                .exists(),
        "must not copy raw app/ .ds/.dsx as the server product: {combined}"
    );
    assert!(
        project
            .path()
            .join("dist")
            .join("client")
            .join("style.css")
            .is_file(),
        "after emit, public/ must copy into dist/client: {combined}"
    );
    // Host extras from the init scaffold (copied into dist/server/).
    assert!(
        project
            .path()
            .join("dist")
            .join("server")
            .join("deka.json")
            .is_file(),
        "build should copy scaffold deka.json into dist/server: {combined}"
    );
    assert!(
        project
            .path()
            .join("dist")
            .join("server")
            .join("deka.lock")
            .is_file(),
        "build should copy scaffold deka.lock into dist/server: {combined}"
    );
}

#[test]
fn build_merges_head_into_dist_html() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "export fn head() {\n    return <title>Head Merge</title>;\n}\nexport fn Page() {\n    return <section><h1>Deka App</h1></section>;\n}\n",
    )
    .expect("write page with head()");

    let (success, combined) = run_build(project.path());
    assert!(success, "deka build should succeed with head(): {combined}");

    let index = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read dist/client/index.html");
    assert!(
        index.contains("Head Merge"),
        "dist HTML should include rendered head(): {index}"
    );
    assert!(
        index.contains("<title"),
        "dist HTML should include a title from head(): {index}"
    );
    assert!(
        !index.contains("<!--deka-head-->"),
        "head hole should be filled: {index}"
    );
}

#[test]
fn build_passes_slug_params_in_generated_entry() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let slug_dir = project.path().join("app").join("blog").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    fs::write(
        slug_dir.join("page.dsx"),
        "interface PageProps { slug: string }\nexport fn Page(props: PageProps) {\n    return <article>{props.slug}</article>;\n}\n",
    )
    .expect("write slug page");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with a [slug] page: {combined}"
    );

    let entry = fs::read_to_string(
        project
            .path()
            .join(".cache")
            .join("dekascript")
            .join("serve-entry.dsx"),
    )
    .expect("read generated serve-entry");
    assert!(
        entry.contains("slug={last_segment(path)}") || entry.contains("slug: last_segment(path)"),
        "generated matcher should pass [slug] into Page: {entry}"
    );
}

#[test]
fn build_bundles_api_handlers_into_worker() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let api_dir = project.path().join("api").join("hello");
    fs::create_dir_all(&api_dir).expect("mkdir api/hello");
    fs::write(
        api_dir.join("route.ds"),
        "interface RequestHeaders { accept: string }\ninterface Request { url: string, pathname: string, method: string, headers: RequestHeaders }\ninterface Response { status: number, body: string }\nexport fn GET(request: Request) Response {\n    return { status: 200, body: \"hello-api\" }\n}\nexport fn POST(request: Request) Response {\n    return { status: 200, body: \"posted\" }\n}\n",
    )
    .expect("write api route");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with api/route.ds: {combined}"
    );

    let worker = fs::read_to_string(project.path().join("dist").join("_worker.js"))
        .expect("read dist/_worker.js");
    assert!(
        worker.contains("hello-api"),
        "worker must include the compiled GET body, not a stub: {worker}"
    );
    assert!(
        worker.contains("posted"),
        "worker must include the compiled POST body: {worker}"
    );
    assert!(
        !worker.contains("API route \" + method"),
        "worker must not be the route-table stub: {worker}"
    );
    assert!(
        worker.contains("export default"),
        "worker must export a Cloudflare fetch handler: {worker}"
    );
}

#[test]
fn build_writes_cloudflare_redirects() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let (success, combined) = run_build(project.path());
    assert!(success, "deka build should succeed: {combined}");
    let redirects = fs::read_to_string(project.path().join("dist").join("_redirects"))
        .expect("read dist/_redirects");
    assert!(
        redirects.contains("/*/ /:splat 301"),
        "default canonical form is no trailing slash: {redirects}"
    );
}

#[test]
fn build_rejects_static_kind_when_api_exists() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let deka = fs::read_to_string(project.path().join("deka.json")).expect("read deka.json");
    let patched = deka.replace(
        "\"serve\": { \"mode\": \"ds\" }",
        "\"serve\": { \"mode\": \"ds\", \"kind\": \"static\" }",
    );
    fs::write(project.path().join("deka.json"), patched).expect("write deka.json");
    let api_dir = project.path().join("api").join("hello");
    fs::create_dir_all(&api_dir).expect("mkdir api");
    fs::write(
        api_dir.join("route.ds"),
        "interface Response { status: number, body: string }\nexport fn GET() Response {\n    return { status: 200, body: \"ok\" }\n}\n",
    )
    .expect("write route");
    let (success, combined) = run_build(project.path());
    assert!(!success, "static kind + api/ must fail: {combined}");
    assert!(
        combined.contains("serve.kind") && combined.contains("worker"),
        "error should name serve.kind: {combined}"
    );
}

#[test]
fn build_desugars_loading_dsx_to_suspense() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("loading.dsx"),
        "export fn Loading() {\n    return <p>Loading...</p>;\n}\n",
    )
    .expect("write loading.dsx");
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with loading.dsx: {combined}"
    );
    let entry = fs::read_to_string(
        project
            .path()
            .join(".cache")
            .join("dekascript")
            .join("serve-entry.dsx"),
    )
    .expect("read generated serve-entry");
    assert!(
        entry.contains("import { Suspense } from \"ui/suspense\""),
        "serve-entry should import Suspense: {entry}"
    );
    assert!(
        entry.contains("<Suspense fallback={<Loading_root />}"),
        "loading.dsx should wrap the child segment as a ComponentNode: {entry}"
    );
}

#[test]
fn build_serve_entry_uses_render_to_stream_for_documents() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let (success, combined) = run_build(project.path());
    assert!(success, "deka build should succeed: {combined}");
    let entry = fs::read_to_string(
        project
            .path()
            .join(".cache")
            .join("dekascript")
            .join("serve-entry.dsx"),
    )
    .expect("read generated serve-entry");
    assert!(
        entry.contains("renderToStreamHtml"),
        "documents should stream via renderToStreamHtml: {entry}"
    );
    assert!(
        entry.contains("renderToStringAsync") && entry.contains("staticBuild"),
        "dist/ prerender should use renderToStringAsync via staticBuild: {entry}"
    );
    assert!(
        entry.contains("async fn App"),
        "App must be async so stream chunks can flush: {entry}"
    );
}

#[test]
fn build_emits_island_chunk_without_server_renderer() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "export fn Counter() {\n    return <button>0</button>;\n}\nexport fn Page() {\n    return <main><Counter client:load count={1} /></main>;\n}\n",
    )
    .expect("write island page");
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with client:load: {combined}"
    );
    assert!(
        combined.contains("island Counter"),
        "build should print an island serialization report: {combined}"
    );
    let assets = project.path().join("dist").join("client").join("assets");
    let chunk = find_hashed_asset(&assets, "islands-load", "js")
        .expect("client:load must emit a hashed islands-load chunk");
    let js = fs::read_to_string(&chunk).expect("read hashed islands-load chunk");
    assert!(
        !js.contains("renderToString") && !js.contains("ui/server"),
        "island chunk must not include the server renderer: {js}"
    );
    let island_mod = fs::read_to_string(
        find_hashed_asset(&assets, "island-load-0", "js")
            .expect("compiled island module must exist under a hashed name"),
    )
    .expect("read compiled island module");
    assert!(
        island_mod.contains("export function Counter") || island_mod.contains("export { Counter }"),
        "compiled island must export Counter: {island_mod}"
    );
    let export_fn = island_mod.matches("export function Counter").count();
    let export_list = island_mod.matches("export { Counter }").count();
    assert!(
        export_fn + export_list == 1,
        "duplicate export of Counter would kill the island chunk: {island_mod}"
    );
    assert!(
        !project
            .path()
            .join("dist")
            .join("client")
            .join("assets")
            .join("ui")
            .join("server.js")
            .is_file(),
        "ui/server.js must not be copied into the client assets"
    );
    let index = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read dist html");
    let chunk_href = format!("/assets/{}", chunk.file_name().unwrap().to_string_lossy());
    assert!(
        index.contains(&chunk_href),
        "html must load the hashed island chunk: {index}"
    );
    assert!(
        !index.contains("/assets/islands-load.js"),
        "html must not reference the unhashed island chunk: {index}"
    );
    let jsx = fs::read_to_string(
        find_hashed_asset(&assets.join("ui"), "jsx", "js").expect("hashed ui/jsx chunk"),
    )
    .expect("read ui/jsx");
    let client = fs::read_to_string(
        find_hashed_asset(&assets.join("ui"), "client", "js").expect("hashed ui/client chunk"),
    )
    .expect("read ui/client");
    let reactive = fs::read_to_string(
        find_hashed_asset(&assets.join("ui"), "reactive", "js").expect("hashed ui/reactive chunk"),
    )
    .expect("read ui/reactive");
    let mut payload = jsx.clone();
    payload.push_str(&client);
    payload.push_str(&reactive);
    payload.push_str(&js);
    if let Some(island_mod) = find_hashed_asset(&assets, "island-load-0", "js") {
        let mod_js = fs::read_to_string(&island_mod).expect("read island module");
        assert!(
            !mod_js.contains("renderToString") && !mod_js.contains("from \"ui/server\""),
            "compiled island module must not import the server renderer: {mod_js}"
        );
        payload.push_str(&mod_js);
    }
    assert!(
        gzip_len(jsx.as_bytes()) < 2_048,
        "ui/jsx gzip budget is 2KiB, got {}",
        gzip_len(jsx.as_bytes())
    );
    assert!(
        gzip_len(payload.as_bytes()) < 16_384,
        "one-button island gzip budget is 16KiB, got {}",
        gzip_len(payload.as_bytes())
    );
}

#[test]
fn build_without_islands_emits_no_island_script() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed without islands: {combined}"
    );
    assert!(
        !combined.contains("island "),
        "build report must not list islands when none exist: {combined}"
    );
    let index = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read dist html");
    assert!(
        !index.contains("islands-load.js")
            && !index.contains("islands-idle.js")
            && !index.contains("islands-visible.js"),
        "a page with no client:* must emit no island script tag: {index}"
    );
    assert!(
        !project
            .path()
            .join("dist")
            .join("client")
            .join("assets")
            .join("ui")
            .join("server.js")
            .is_file(),
        "ui/server.js must not be copied when there are no islands"
    );
}

#[test]
fn build_emits_per_route_css_into_head() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "export fn Page() {\n    return <section class=\"p-4 text-lg\">Home</section>;\n}\n",
    )
    .expect("write home page");
    let about_dir = project.path().join("app").join("about");
    fs::create_dir_all(&about_dir).expect("mkdir about");
    fs::write(
        about_dir.join("page.dsx"),
        "export fn Page() {\n    return <section class=\"p-4 text-sm\">About</section>;\n}\n",
    )
    .expect("write about page");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with per-route CSS: {combined}"
    );

    let css_dir = project
        .path()
        .join("dist")
        .join("client")
        .join("assets")
        .join("css");
    let home = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read home html");
    let common_name = hashed_asset_name_in(&css_dir, "common", "css");
    assert!(
        home.contains(&format!("/assets/css/{common_name}")),
        "shared classes should hoist to a hashed common.css: {home}"
    );
    assert!(
        !home.contains("/assets/css/common.css"),
        "home must not reference the unhashed common.css: {home}"
    );
    let root_name = hashed_asset_name_in(&css_dir, "route-root", "css");
    assert!(
        home.contains(&format!("/assets/css/{root_name}")),
        "home unique classes should be a hashed route stylesheet: {home}"
    );
    let common = fs::read_to_string(css_dir.join(&common_name)).expect("read common.css");
    assert!(
        common.contains(".p-4"),
        "shared p-4 must land in common.css: {common}"
    );
    let root_css = fs::read_to_string(css_dir.join(&root_name)).expect("read route-root.css");
    assert!(
        root_css.contains(".text-lg"),
        "home-only class must land in route CSS: {root_css}"
    );
    assert!(
        !root_css.contains(".text-sm"),
        "about-only class must not leak into home CSS: {root_css}"
    );
    let about = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("about")
            .join("index.html"),
    )
    .expect("read about html");
    let about_name = hashed_asset_name_in(&css_dir, "route-about", "css");
    assert!(
        about.contains(&format!("/assets/css/{about_name}")),
        "about unique classes should be a hashed route stylesheet: {about}"
    );
}

#[test]
fn build_scopes_component_css_with_the_modules_cid() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let page_src = "import \"./page.css\"\nexport fn Page() {\n    return <section class=\"greeting\">Home</section>;\n}\n";
    fs::write(project.path().join("app").join("page.dsx"), page_src).expect("write home page");
    fs::write(
        project.path().join("app").join("page.css"),
        ".greeting { color: rebeccapurple; }\n",
    )
    .expect("write component css");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with component CSS: {combined}"
    );

    // The compiler stamps the page's host elements with
    // `data-deka-cid-<hash>` (hash of the module source) and the CSS writer
    // rewrites the component's selectors to require it (RFD 24 §10.6).
    let cid = runtime_core::framework::css_scope_hash(page_src);
    let css_dir = project
        .path()
        .join("dist")
        .join("client")
        .join("assets")
        .join("css");
    let root_name = hashed_asset_name_in(&css_dir, "route-root", "css");
    let root_css = fs::read_to_string(css_dir.join(&root_name)).expect("read route-root.css");
    assert!(
        root_css.contains(&format!(".greeting[data-deka-cid-{cid}]")),
        "component selector must require the module's scope stamp: {root_css}"
    );

    // A style-free page emits no scoped CSS (no dead weight).
    let about_dir = project.path().join("app").join("about");
    fs::create_dir_all(&about_dir).expect("mkdir about");
    fs::write(
        about_dir.join("page.dsx"),
        "export fn Page() {\n    return <section>About</section>;\n}\n",
    )
    .expect("write about page");
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "rebuild with a style-free page should succeed: {combined}"
    );
    // #604 content-addresses route stylesheets, so a style-free route emits
    // no route-about.<hash>.css at all; if one exists it must carry no stamp.
    let css_dir = project
        .path()
        .join("dist")
        .join("client")
        .join("assets")
        .join("css");
    if let Some(about_css_path) = find_hashed_asset(&css_dir, "route-about", "css") {
        let about_css = fs::read_to_string(&about_css_path).expect("read route-about css");
        assert!(
            !about_css.contains("data-deka-cid"),
            "a style-free page must not emit scoped CSS: {about_css}"
        );
    }
}

#[test]
fn build_fails_loudly_on_unscopeable_component_css() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "import \"./reset.css\"\nexport fn Page() {\n    return <section>Home</section>;\n}\n",
    )
    .expect("write home page");
    fs::write(
        project.path().join("app").join("reset.css"),
        "body { margin: 0; }\n",
    )
    .expect("write document-level css");

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "build must fail when component CSS targets the document: {combined}"
    );
    assert!(
        combined.contains("cannot be scoped"),
        "the diagnostic must name the scoping violation: {combined}"
    );
}

#[test]
fn build_server_defer_requires_fallback_and_emits_loader() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "export fn Badge() {\n    return <strong>42</strong>;\n}\nexport fn Page() {\n    return <main><Badge server:defer cache=\"60s\"><span slot=\"fallback\">.</span></Badge></main>;\n}\n",
    )
    .expect("write defer page");
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with server:defer fallback: {combined}"
    );
    let assets_dir = project.path().join("dist").join("client").join("assets");
    let defer_name = hashed_asset_name_in(&assets_dir, "islands-defer", "js");
    let index = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read dist html");
    assert!(
        index.contains(&format!("/assets/{defer_name}")),
        "server:defer must emit the hashed defer loader: {index}"
    );
    assert!(
        !index.contains("/assets/islands-defer.js"),
        "dist html must not reference the unhashed defer loader: {index}"
    );
    assert!(
        !index.contains("<strong>42</strong>"),
        "static shell must not include the deferred tree: {index}"
    );
    assert!(
        assets_dir.join(&defer_name).is_file(),
        "hashed defer loader must exist: {defer_name}"
    );
}

#[test]
fn build_server_defer_without_fallback_fails() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "export fn Badge() {\n    return <strong>42</strong>;\n}\nexport fn Page() {\n    return <main><Badge server:defer /></main>;\n}\n",
    )
    .expect("write defer page without fallback");
    let (success, combined) = run_build(project.path());
    assert!(!success, "missing fallback must fail the build: {combined}");
    assert!(
        combined.contains("slot=\"fallback\"") || combined.contains("fallback"),
        "error should mention the required fallback slot: {combined}"
    );
}

#[test]
fn build_worker_dispatches_defer_and_copies_headers() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "export fn Badge() {\n    return <strong>42</strong>;\n}\nexport fn Page() {\n    return <main><Badge server:defer cache=\"60s\"><span slot=\"fallback\">.</span></Badge></main>;\n}\n",
    )
    .expect("write defer page");
    fs::create_dir_all(project.path().join("public")).expect("mkdir public");
    fs::write(project.path().join("public").join("ok.css"), "body{}").expect("write public css");
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with server:defer: {combined}"
    );
    let worker = fs::read_to_string(project.path().join("dist").join("_worker.js"))
        .expect("read dist/_worker.js");
    assert!(
        worker.contains("/_deka/defer"),
        "worker must dispatch the defer endpoint: {worker}"
    );
    assert!(
        worker.contains("isGetHead") || worker.contains("method === \"GET\""),
        "worker must not 301 POST /_deka/defer: {worker}"
    );
    assert!(
        worker.contains("request.text()") || worker.contains("await request.text()"),
        "worker must read POST bodies: {worker}"
    );
    assert!(
        worker.contains("ok.css") || worker.contains("/ok.css"),
        "worker must serve public files ahead of the api router: {worker}"
    );
    assert!(
        worker.contains("for (const key of Object.keys(rawHeaders))")
            || worker.contains("Object.keys(rawHeaders)"),
        "worker must copy response headers: {worker}"
    );
}

#[test]
fn build_trailing_slash_true_does_not_loop_redirects() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let deka = fs::read_to_string(project.path().join("deka.json")).expect("read deka.json");
    let patched = deka.replace(
        "\"serve\": { \"mode\": \"ds\" }",
        "\"serve\": { \"mode\": \"ds\", \"trailingSlash\": true }",
    );
    fs::write(project.path().join("deka.json"), patched).expect("write deka.json");
    let (success, combined) = run_build(project.path());
    assert!(success, "deka build should succeed: {combined}");
    let redirects = fs::read_to_string(project.path().join("dist").join("_redirects"))
        .expect("read dist/_redirects");
    assert!(
        !redirects.contains("/* /:splat/"),
        "add-slash splat loops on Cloudflare: {redirects}"
    );
}

/// Every file name under `dir` (recursively), for content-addressing checks.
fn all_asset_names(dir: &Path) -> BTreeSet<String> {
    fn walk(dir: &Path, out: &mut BTreeSet<String>) {
        let Ok(reader) = fs::read_dir(dir) else {
            return;
        };
        for entry in reader.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(dir, &mut out);
    out
}

/// The URL `key` maps to in `<dir>/importmap.json`.
fn importmap_url(dir: &Path, key: &str) -> String {
    let raw = fs::read_to_string(dir.join("importmap.json")).expect("read importmap.json");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("parse importmap.json");
    value["imports"][key]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn build_island_assets_are_content_addressed() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "export fn Counter() {\n    return <button>0</button>;\n}\nexport fn Page() {\n    return <main><Counter client:load count={1} /></main>;\n}\n",
    )
    .expect("write island page");

    let (success, combined) = run_build(project.path());
    assert!(success, "first build should succeed: {combined}");
    let assets = project.path().join("dist").join("client").join("assets");
    let first = all_asset_names(&assets);

    let (success, combined) = run_build(project.path());
    assert!(success, "second build should succeed: {combined}");
    let second = all_asset_names(&assets);
    assert_eq!(
        first, second,
        "rebuilding without source changes must reproduce identical hashed names"
    );
}

#[test]
fn build_island_change_rotates_hash_and_importmap() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let page = project.path().join("app").join("page.dsx");
    fs::write(
        &page,
        "export fn Counter() {\n    return <button>0</button>;\n}\nexport fn Page() {\n    return <main><Counter client:load count={1} /></main>;\n}\n",
    )
    .expect("write island page");

    let (success, combined) = run_build(project.path());
    assert!(success, "first build should succeed: {combined}");
    let assets = project.path().join("dist").join("client").join("assets");
    let first_url = importmap_url(&assets, "islands/load");
    assert!(
        first_url.starts_with("/assets/islands-load.") && first_url.ends_with(".js"),
        "importmap must map islands/load to a hashed URL: {first_url}"
    );
    let index = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read dist html");
    assert!(
        index.contains(&first_url),
        "dist html must reference the importmap URL: {index}"
    );

    fs::write(
        &page,
        "export fn Counter() {\n    return <button>1</button>;\n}\nexport fn Page() {\n    return <main><Counter client:load count={1} /></main>;\n}\n",
    )
    .expect("change island source");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "rebuild after an island change should succeed: {combined}"
    );
    let second_url = importmap_url(&assets, "islands/load");
    assert_ne!(
        first_url, second_url,
        "changing the island must rotate its content hash"
    );
    let first_name = first_url.trim_start_matches("/assets/").to_string();
    assert!(
        !assets.join(&first_name).exists(),
        "stale chunk must be cleaned after the hash rotates: {first_name}"
    );
    let index = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read dist html");
    assert!(
        index.contains(&second_url),
        "dist html must reference the new hashed URL: {index}"
    );
    assert!(
        !index.contains(&first_url),
        "dist html must not reference the stale hashed URL: {index}"
    );
}

fn gzip_len(bytes: &[u8]) -> usize {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("gzip write");
    encoder.finish().expect("gzip finish").len()
}
