//! Asset-fetch boundary test for RFD 24 §10.7 content-hashed client assets
//! (deka#604 serve-side follow-up).
//!
//! Client assets are content-addressed (`<stem>.<sha256-10>.js|css`), so every
//! URL a document or chunk references must agree with the name the writer
//! emitted on disk. This test pins the three URL-agreement pairs at once by
//! fetching, over real HTTP against `deka serve`, every reference found in:
//!
//! 1. the served route HTML — every `<script src>` and `<link href>` under
//!    `/assets/` (router.rs output ↔ islands.rs/css.rs writers),
//! 2. `assets/importmap.json` — every URL it maps a specifier to
//!    (importmap ↔ writers),
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
fn write_boundary_fixture(dir: &Path) {
    init_project(dir);
    fs::write(
        dir.join("app").join("page.dsx"),
        "export fn Counter() {\n    return <button class=\"p-4 text-lg\">0</button>;\n}\nexport fn Page() {\n    return <main><Counter client:load count={1} /></main>;\n}\n",
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
/// (covers `<script src="/assets/...">`, `<link href="/assets/...">`, and
/// `<script type="importmap" src="/assets/importmap.json">`).
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
    assert!(
        html.contains("type=\"importmap\"") && html.contains("/assets/importmap.json"),
        "served document must wire the client import map: {html}\n{context}"
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
}
