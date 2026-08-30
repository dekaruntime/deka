// Regression test for deka#5: `deka build` must fail closed (non-zero exit,
// no dist/ output) when the web project's source under app/ does not
// compile -- matching what `deka check` already reports for that file.
//
// Root cause (see runtime/crates/cli/src/cli/build.rs): the web-project
// build path only ever compiled `serve.entry`, and only when the entry
// used a hydration component; every other .ds file under app/, including
// the entry itself in the common non-hydration case, was copied into
// dist/server/app as raw, unvalidated bytes via copy_dir_recursive. A
// project with syntactically invalid source therefore "built" successfully.

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
        project.path().join("dist").join("client").join("index.html"),
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
    assert!(
        project.path().join("dist").join("server").join("app").join("page.dsx").is_file(),
        "successful build should copy app/ into dist/server/app: {combined}"
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

    let index = fs::read_to_string(project.path().join("dist").join("client").join("index.html"))
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
        entry.contains("slug: last_segment(path)"),
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
        "interface RequestHeaders { accept: string }\ninterface Request { url: string, pathname: string, method: string, headers: RequestHeaders }\ninterface Response { status: number, body: string }\nexport fn GET(request: Request): Response {\n    return { status: 200, body: \"hello-api\" }\n}\nexport fn POST(request: Request): Response {\n    return { status: 200, body: \"posted\" }\n}\n",
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
        "interface Response { status: number, body: string }\nexport fn GET(): Response {\n    return { status: 200, body: \"ok\" }\n}\n",
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
fn build_emits_worker_for_middleware() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("middleware.ds"),
        "export const matcher = [\"/dashboard/:path*\"]\ninterface RequestHeaders { accept: string }\ninterface ResponseHeaders { location: string }\ninterface Request { url: string, pathname: string, method: string, headers: RequestHeaders }\ninterface Response { status: number, body: string, headers: ResponseHeaders }\nexport fn middleware(request: Request): Option<Response> {\n    return Some({ status: 302, body: \"\", headers: { location: \"/login\" } })\n}\n",
    )
    .expect("write middleware.ds");
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with middleware.ds: {combined}"
    );
    let worker = fs::read_to_string(project.path().join("dist").join("_worker.js"))
        .expect("read dist/_worker.js");
    assert!(
        worker.contains("/login"),
        "worker must compile middleware redirect: {worker}"
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
    assert!(success, "deka build should succeed with loading.dsx: {combined}");
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
        entry.contains("Suspense({ fallback: Loading_root()"),
        "loading.dsx should wrap the child segment: {entry}"
    );
}
