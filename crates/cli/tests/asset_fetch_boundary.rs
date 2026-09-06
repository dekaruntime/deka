//! Asset-fetch boundary test for RFD 24 §10.7 content-hashed client assets
//! (deka#604 serve-side follow-up).
//!
//! Client assets are content-addressed (`<stem>.<sha256-10>.js|css`), so every
//! URL a document or chunk references must agree with the name the writer
//! emitted on disk. This test pins the three URL-agreement pairs at once by
//! fetching, over real HTTP against `deka serve`, every reference found in:
//!
//! 1. the served route HTML — every `<script src>` and `<link href>` under
//!    `/assets/` plus every URL in the inline import map (router.rs output ↔
//!    islands.rs/css.rs writers),
//! 2. `assets/importmap.json` — every URL it maps a specifier to
//!    (importmap ↔ writers; the on-disk copy the inline tag is built from),
//! 3. the served/emitted JS chunks themselves — every relative import target
//!    (`./ui/jsx.<hash>.js`, `./island-load-0.<hash>.js`, ...) resolved
//!    against the importing chunk's URL (chunks ↔ writers).
//!
//! Every reference must return 200 with a non-empty body; a single 404 fails
//! the test. A second half builds the same fixture with `deka build` and
//! asserts every reference in the dist HTML exists on disk — and that the
//! dev-mode (serve) and prod-mode (build) URL sets are identical for the
//! same source (dev/prod parity).

use std::collections::BTreeSet;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::redirect;
use tempfile::TempDir;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

struct Serve {
    child: Child,
    port: u16,
    root: TempDir,
}

impl Serve {
    fn log(&self) -> String {
        fs::read_to_string(self.root.path().join("serve.log")).unwrap_or_default()
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral")
        .local_addr()
        .expect("local addr")
        .port()
}

fn client() -> Client {
    Client::builder()
        .redirect(redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client")
}

fn init_project(dir: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(dir)
        .output()
        .expect("deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// App-router fixture with a client island (hashed JS chunks + importmap)
/// and shared/unique utility classes on two routes (hashed CSS). Same source
/// feeds both the serve and the build half so URL sets must match exactly.
///
/// The island imports `ui/form` and `ui/suspense` so the rewrite's targets
/// are in the live HTTP closure — deka#622 finding E. A 404 here is the
/// original bug (Form / Suspense inside an island never ran).
fn write_boundary_fixture(dir: &Path) {
    init_project(dir);
    fs::write(
        dir.join("app").join("page.dsx"),
        concat!(
            "import { Form } from \"ui/form\"\n",
            "import { Suspense } from \"ui/suspense\"\n",
            "export fn Counter() {\n",
            "    return <Suspense fallback={<span>0</span>}><Form action={\"/\"} method={\"post\"}><button class=\"p-4 text-lg\">0</button></Form></Suspense>;\n",
            "}\n",
            "export fn Page() {\n",
            "    return <main><Counter client:load count={1} /></main>;\n",
            "}\n",
        ),
    )
    .expect("write island page");
    let about = dir.join("app").join("about");
    fs::create_dir_all(&about).expect("mkdir about");
    fs::write(
        about.join("page.dsx"),
        "export fn Page() {\n    return <section class=\"p-4 text-sm\">About</section>;\n}\n",
    )
    .expect("write about page");
}

fn spawn_serve() -> Serve {
    let root = tempfile::tempdir().expect("tempdir");
    write_boundary_fixture(root.path());
    let port = free_port();
    let log_path = root.path().join("serve.log");
    let log = fs::File::create(&log_path).expect("serve.log");
    let child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root.path())
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka serve");
    let serve = Serve { child, port, root };
    wait_ready(serve.port, &serve);
    serve
}

fn wait_ready(port: u16, serve: &Serve) {
    let http = client();
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if let Ok(res) = http.get(format!("http://127.0.0.1:{port}/")).send() {
            if res.status().as_u16() == 200 {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!(
        "deka serve did not become ready on port {port}\nserve.log:\n{}",
        serve.log()
    );
}

/// Every `/assets/...` URL appearing as a double-quoted string in `text`
/// (covers `<script src="/assets/...">`, `<link href="/assets/...">`, and the
/// URLs inside an inline `<script type="importmap">{...}</script>` body).
fn quoted_asset_urls(text: &str) -> BTreeSet<String> {
    text.split('"')
        .filter_map(|seg| {
            seg.strip_prefix("/assets/")
                .map(|rest| format!("/assets/{rest}"))
        })
        .collect()
}

/// Relative `./...` import targets inside an emitted JS chunk
/// (`from "./ui/jsx.<hash>.js"`, `import("./x.<hash>.js")`).
fn relative_import_targets(js: &str) -> Vec<String> {
    js.split('"')
        .filter_map(|seg| seg.strip_prefix("./").map(str::to_string))
        .collect()
}

/// Resolve a chunk-relative import target against the importing chunk's URL.
fn resolve_relative(importer: &str, target: &str) -> String {
    let base = importer.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
    format!("{base}/{target}")
}

fn importmap_urls(body: &[u8]) -> BTreeSet<String> {
    let map: serde_json::Value = serde_json::from_slice(body).expect("importmap.json must parse");
    map.get("imports")
        .and_then(|imports| imports.as_object())
        .expect("importmap.json must have an imports object")
        .values()
        .filter_map(|v| v.as_str())
        .filter(|url| url.starts_with("/assets/"))
        .map(str::to_string)
        .collect()
}

/// Extract the inline import map from a served or built document. Browsers
/// reject `<script type="importmap" src=...>` — the attribute is disallowed on
/// the element — so the document must carry the JSON inline. Asserts the tag
/// exists, has no `src` attribute anywhere, and that its text content parses
/// as JSON with a non-empty `imports` object. Returns the imports map.
fn inline_importmap(html: &str, context: &str) -> serde_json::Map<String, serde_json::Value> {
    assert!(
        !html.contains(r#"<script type="importmap" src="#),
        "document must not reference the import map by URL (browsers reject the src form)\n{context}"
    );
    let open = r#"<script type="importmap">"#;
    let start = html
        .find(open)
        .unwrap_or_else(|| panic!("document must carry an inline import map\n{context}"));
    let body = &html[start + open.len()..];
    let end = body
        .find("</script>")
        .expect("inline import map tag must close");
    let map: serde_json::Value =
        serde_json::from_str(&body[..end]).expect("inline import map body must parse as JSON");
    map.get("imports")
        .and_then(|imports| imports.as_object())
        .filter(|imports| !imports.is_empty())
        .expect("inline import map must have a non-empty imports object")
        .clone()
}

/// Fetch-closure over live HTTP: start from a document's references, then
/// walk importmap URLs and chunk-internal relative imports. Asserts every
/// reference returns 200 with a non-empty body. Returns the resolved set.
fn collect_live_refs(http: &Client, base: &str, html: &str, context: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = quoted_asset_urls(html).into_iter().collect();
    while let Some(url) = queue.pop() {
        if !seen.insert(url.clone()) {
            continue;
        }
        let res = http
            .get(format!("{base}{url}"))
            .send()
            .unwrap_or_else(|err| panic!("GET {url} failed: {err}\n{context}"));
        let status = res.status().as_u16();
        let body = res.bytes().expect("read body");
        assert_eq!(
            status,
            200,
            "{url} must resolve, got {status}\nbody: {}\n{context}",
            String::from_utf8_lossy(&body)
        );
        assert!(
            !body.is_empty(),
            "{url} must have a non-empty body\n{context}"
        );
        if url.ends_with(".js") {
            let js = String::from_utf8_lossy(&body);
            for target in relative_import_targets(&js) {
                queue.push(resolve_relative(&url, &target));
            }
        }
        if url.ends_with("importmap.json") {
            queue.extend(importmap_urls(&body));
        }
    }
    seen
}

/// On-disk walk for the dist half: same closure, but every reference must
/// exist as a non-empty file under `doc_root` (no server needed).
fn collect_disk_refs(doc_root: &Path, html: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = quoted_asset_urls(html).into_iter().collect();
    while let Some(url) = queue.pop() {
        if !seen.insert(url.clone()) {
            continue;
        }
        let rel = url.trim_start_matches('/');
        let path = doc_root.join(rel);
        let body = fs::read(&path)
            .unwrap_or_else(|err| panic!("{url} must exist on disk at {}: {err}", path.display()));
        assert!(!body.is_empty(), "{url} must be non-empty on disk");
        if url.ends_with(".js") {
            let js = String::from_utf8_lossy(&body);
            for target in relative_import_targets(&js) {
                queue.push(resolve_relative(&url, &target));
            }
        }
        if url.ends_with("importmap.json") {
            queue.extend(importmap_urls(&body));
        }
    }
    seen
}

/// The unhashed logical names must never survive into a served or built
/// document — the .cache/dist assets are hashed-only, so these would 404.
fn assert_no_unhashed_refs(html: &str, context: &str) {
    for logical in [
        "/assets/islands-load.js",
        "/assets/islands-idle.js",
        "/assets/islands-visible.js",
        "/assets/islands-defer.js",
        "/assets/css/common.css",
        "/assets/css/route-root.css",
        "/assets/css/route-about.css",
    ] {
        assert!(
            !html.contains(logical),
            "document must not reference unhashed {logical}\n{context}"
        );
    }
}

fn hashed_name_present(html: &str, stem: &str) -> bool {
    let needle = format!("/assets/{stem}.");
    html.split('"').any(|seg| {
        seg.strip_prefix(&needle)
            .map(|rest| rest.ends_with(".js") || rest.ends_with(".css"))
            .unwrap_or(false)
    })
}

#[test]
fn serve_asset_references_all_resolve() {
    let serve = spawn_serve();
    let http = client();
    let base = format!("http://127.0.0.1:{}", serve.port);
    let context = format!("serve.log:\n{}", serve.log());

    let html = http
        .get(format!("{base}/"))
        .send()
        .expect("GET /")
        .text()
        .expect("read / html");

    assert_no_unhashed_refs(&html, &context);
    let imports = inline_importmap(&html, &context);
    // The dev HMR client (crates/http/src/router.rs) imports the LOGICAL
    // specifier "ui/client"; that specifier must be a key in this map,
    // mapped to the hashed chunk, or the dev client's dynamic import fails
    // silently and islands stop re-hydrating (deka#596 follow-up).
    let ui_client = imports["ui/client"]
        .as_str()
        .expect("ui/client must be a key in the inline import map");
    assert!(
        ui_client.starts_with("/assets/ui/client.") && ui_client.ends_with(".js"),
        "ui/client must map to its hashed chunk: {ui_client}"
    );
    let res = http
        .get(format!("{base}{ui_client}"))
        .send()
        .unwrap_or_else(|err| panic!("GET {ui_client} failed: {err}\n{context}"));
    assert_eq!(
        res.status().as_u16(),
        200,
        "the ui/client chunk the dev client resolves to must be served: {ui_client}\n{context}"
    );
    for stem in ["islands-load", "css/common", "css/route-root"] {
        assert!(
            hashed_name_present(&html, stem),
            "served document must reference the hashed {stem} asset: {html}\n{context}"
        );
    }

    let refs = collect_live_refs(&http, &base, &html, &context);
    assert!(
        refs.len() >= 6,
        "expected the full asset closure (islands chunk, island module, ui chunks, css, importmap), got {refs:?}\n{context}"
    );
    // deka#622 finding E: Form / Suspense inside an island must resolve.
    // collect_live_refs already 404s a missing URL; these asserts pin that
    // the island's rewritten imports actually entered the closure.
    for stem in ["ui/form", "ui/suspense"] {
        assert!(
            refs.iter()
                .any(|url| url.contains(&format!("/{stem}.")) && url.ends_with(".js")),
            "island import of {stem} must resolve to a hashed chunk: {refs:?}\n{context}"
        );
    }
    // The second route contributes its own per-route stylesheet.
    let about = http
        .get(format!("{base}/about"))
        .send()
        .expect("GET /about")
        .text()
        .expect("read /about html");
    assert_no_unhashed_refs(&about, &context);
    let about_refs = collect_live_refs(&http, &base, &about, &context);
    assert!(
        about_refs.iter().any(|url| url.contains("route-about.")),
        "/about must reference its hashed route stylesheet: {about_refs:?}\n{context}"
    );
}

#[test]
fn serve_and_build_resolve_identical_asset_urls() {
    // Same fixture source, two tempdirs: one served, one built. The resolved
    // /assets URL sets must be identical — dev/prod parity by construction.
    let serve = spawn_serve();
    let http = client();
    let base = format!("http://127.0.0.1:{}", serve.port);
    let live_context = format!("serve.log:\n{}", serve.log());
    let served_html = http
        .get(format!("{base}/"))
        .send()
        .expect("GET /")
        .text()
        .expect("read served html");
    let live_refs = collect_live_refs(&http, &base, &served_html, &live_context);

    let project = tempfile::tempdir().expect("tempdir");
    write_boundary_fixture(project.path());
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(project.path())
        .output()
        .expect("run deka build");
    assert!(
        output.status.success(),
        "deka build failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let dist_client = project.path().join("dist").join("client");
    let dist_html = fs::read_to_string(dist_client.join("index.html")).expect("read dist html");
    let disk_refs = collect_disk_refs(&dist_client, &dist_html);

    // The map the browser consults (inline, not the on-disk copy) must be
    // identical in dev and prod.
    let live_imports = inline_importmap(&served_html, &live_context);
    let dist_imports = inline_importmap(&dist_html, "dist index.html");
    assert_eq!(
        live_imports, dist_imports,
        "dev (serve) and prod (build) must inline the same import map.\n{live_context}"
    );

    assert_eq!(
        live_refs, disk_refs,
        "dev (serve) and prod (build) must resolve the same source to the same /assets URLs.\nlive: {live_refs:?}\ndist:  {disk_refs:?}\n{live_context}"
    );
}

#[test]
fn dist_asset_references_exist_on_disk() {
    let project = tempfile::tempdir().expect("tempdir");
    write_boundary_fixture(project.path());
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(project.path())
        .output()
        .expect("run deka build");
    assert!(
        output.status.success(),
        "deka build failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let dist_client = project.path().join("dist").join("client");
    for doc in ["index.html", "about/index.html"] {
        let html = fs::read_to_string(dist_client.join(doc)).expect("read dist html");
        assert_no_unhashed_refs(&html, doc);
        let refs = collect_disk_refs(&dist_client, &html);
        assert!(
            !refs.is_empty(),
            "{doc} must reference at least one hashed asset"
        );
    }
    // The import map the browser loads must agree with the chunks on disk.
    let importmap = fs::read(dist_client.join("assets").join("importmap.json"))
        .expect("dist importmap.json must exist");
    for url in importmap_urls(&importmap) {
        let path = dist_client.join(url.trim_start_matches('/'));
        assert!(
            path.is_file(),
            "importmap URL {url} must exist on disk at {}",
            path.display()
        );
    }
    // The map inlined into the document is what the browser consults; it must
    // agree with the on-disk copy and carry no `src` reference.
    let index_html = fs::read_to_string(dist_client.join("index.html")).expect("read dist html");
    let inline = inline_importmap(&index_html, "dist index.html");
    let disk: serde_json::Value = serde_json::from_slice(&importmap).expect("parse importmap.json");
    let disk_imports = disk
        .get("imports")
        .and_then(|v| v.as_object())
        .expect("importmap.json must have an imports object");
    assert_eq!(
        &inline, disk_imports,
        "inline import map must agree with assets/importmap.json"
    );
}

// ---------------------------------------------------------------------------
// deka#622 finding D: import-map prefix skew.
//
// The browser import map's stdlib prefixes must point at the layout
// `deka install` actually writes — `ds_modules/@deka/<pkg>/`, the same
// scoped alias the server resolvers derive from `module_spec_aliases`.
// deka#604 inlined the map into the document, so a stale prefix is a real
// 404 rather than a theoretical one.
// ---------------------------------------------------------------------------

/// A `serve.entry` web project with hydration enabled and one scoped package
/// installed per stdlib prefix — exactly the tree `deka install` writes
/// (crates/cli/tests/project_gate_boundary.rs lays it out the same way, in
/// lieu of a registry to install from).
fn write_importmap_prefix_fixture(dir: &Path) {
    fs::create_dir_all(dir.join("app")).expect("mkdir app");
    fs::create_dir_all(dir.join("public")).expect("mkdir public");
    fs::write(
        dir.join("deka.json"),
        concat!(
            r#"{"name":"prefix-boundary","type":"serve","#,
            r#""dependencies":{"@deka/component":"*","@deka/db":"*","@deka/deka":"*","@deka/encoding":"*"},"#,
            r#""serve":{"entry":"app/main.dsx"}}"#
        ),
    )
    .expect("write deka.json");
    fs::write(
        dir.join("deka.lock"),
        r#"{"lockfileVersion":1,"packages":{}}"#,
    )
    .expect("write deka.lock");
    fs::write(
        dir.join("index.html"),
        "<!doctype html>\n<html><head><title>prefix</title></head><body><div id=\"app\"></div></body></html>\n",
    )
    .expect("write index.html");
    // Hydration enables the client compile whose entry keeps the bare
    // `encoding/json` import for the browser to resolve through the map.
    fs::write(
        dir.join("app").join("main.dsx"),
        concat!(
            "import { shout } from \"encoding/json\"\n\n",
            "fn Hydration() {\n    return <span>hydrate</span>\n}\n\n",
            "export fn App() {\n    const label = shout(\"hi\")\n",
            "    return <div><Hydration/><p>{label}</p></div>\n}\n",
        ),
    )
    .expect("write entry");
    for package in ["component", "db", "deka", "encoding"] {
        let module_dir = dir.join("ds_modules").join("@deka").join(package);
        fs::create_dir_all(&module_dir).expect("module dir");
        // The entry imports "encoding/json", which resolves to
        // @deka/encoding/json — the subpath, not the package index.
        let (source, export) = if package == "encoding" {
            (module_dir.join("json.ds"), "shout")
        } else {
            (module_dir.join("index.ds"), "stub")
        };
        fs::write(
            source,
            format!("export fn {export}(value: string) string {{\n    return value\n}}\n"),
        )
        .expect("module source");
    }
}

/// Every `/ds_modules/` prefix URL in every emitted import map must resolve
/// to a path that exists under dist/client — the static root the browser
/// fetches from. This fails on the pre-fix map twice over: the prefixes
/// point at the unscoped layout (`/ds_modules/encoding/`) while installs
/// land under `@deka/`, and dist/client never received the module tree at
/// all. Reachability, not string equality, so the next layout drift fails
/// here instead of 404ing in a browser.
#[test]
fn dist_importmap_stdlib_prefixes_match_installed_packages() {
    let project = tempfile::tempdir().expect("tempdir");
    write_importmap_prefix_fixture(project.path());
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(project.path())
        .output()
        .expect("run deka build");
    assert!(
        output.status.success(),
        "deka build failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let dist_client = project.path().join("dist").join("client");
    // deka#624: this is the web-bootstrap (`serve.entry`) path, not app-router.
    // The document must carry the map inline — browsers reject `src=` on
    // `<script type="importmap">`.
    let index_html = fs::read_to_string(dist_client.join("index.html")).expect("read dist html");
    let inline = inline_importmap(&index_html, "web-bootstrap dist index.html");
    // Both copies the build emits: the assets-side original and the
    // document-root copy kept for tooling. HTML must not reference either by URL.
    for map_path in [
        dist_client.join("assets").join("importmap.json"),
        dist_client.join("importmap.json"),
    ] {
        let map: serde_json::Value =
            serde_json::from_slice(&fs::read(&map_path).expect("read importmap"))
                .expect("importmap must parse");
        let imports = map
            .get("imports")
            .and_then(|v| v.as_object())
            .expect("importmap must have an imports object");
        let prefixes: Vec<&str> = imports
            .values()
            .filter_map(|url| url.as_str())
            .filter(|url| url.starts_with("/ds_modules/"))
            .collect();
        assert!(
            !prefixes.is_empty(),
            "{} must carry /ds_modules/ stdlib prefixes: {imports:?}",
            map_path.display()
        );
        for url in prefixes {
            let path = dist_client.join(url.trim_start_matches('/'));
            assert!(
                path.exists(),
                "import map prefix {url} (in {}) must resolve to a path that exists under dist/client: {}",
                map_path.display(),
                path.display()
            );
        }
    }
    let disk: serde_json::Value = serde_json::from_slice(
        &fs::read(dist_client.join("assets").join("importmap.json")).expect("read assets map"),
    )
    .expect("parse assets map");
    let disk_imports = disk
        .get("imports")
        .and_then(|v| v.as_object())
        .expect("assets importmap must have imports");
    assert_eq!(
        &inline, disk_imports,
        "inlined web-bootstrap import map must agree with assets/importmap.json"
    );
}
