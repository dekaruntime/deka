//! Asset-fetch boundary coverage for the surviving public asset pipeline.
//!
//! The framework extraction (deka#893) removes app-router prerendering,
//! island chunks, and generated utility CSS. Public assets still pass through
//! source `deka serve` and `deka build`. Keep real HTTP/disk closure checks and
//! serve/build URL parity for that path, including content-hashed nested JS,
//! stylesheets, and inline/on-disk import maps. The web-bootstrap prefix test
//! below separately covers the compiler-emitted import map.

use std::collections::BTreeSet;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use deka_host::integrity::compute_package_integrity;
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

/// Supply public documents so these tests do not require the paused JSX
/// renderer. Asset names are real content hashes, and nested imports ensure
/// the fetch walk checks more than the document's direct references.
fn write_boundary_fixture(dir: &Path) {
    init_project(dir);
    let public = dir.join("public");
    let assets = public.join("assets");
    fs::create_dir_all(assets.join("chunks")).expect("mkdir chunks");
    fs::create_dir_all(assets.join("css")).expect("mkdir css");
    fs::create_dir_all(public.join("about")).expect("mkdir about");

    let value = write_hashed_asset(&assets, "chunks/value.js", "export const value = 42;\n");
    let widget = write_hashed_asset(
        &assets,
        "chunks/widget.js",
        &format!(
            "import {{ value }} from \"./{}\";\nexport {{ value }};\n",
            value.rsplit('/').next().unwrap()
        ),
    );
    let main = write_hashed_asset(
        &assets,
        "main.js",
        &format!("import {{ value }} from \"./{widget}\";\nconsole.log(value);\n"),
    );
    let common = write_hashed_asset(&assets, "css/common.css", "body { margin: 0; }\n");
    let mapped = write_hashed_asset(&assets, "mapped.js", "export const mapped = true;\n");
    let map = serde_json::json!({"imports": {
        "boundary/main": format!("/assets/{main}"),
        "boundary/mapped": format!("/assets/{mapped}")
    }});
    fs::write(assets.join("importmap.json"), map.to_string()).expect("write importmap");
    for (doc, route) in [("boundary.html", "root"), ("about/boundary.html", "about")] {
        let css = write_hashed_asset(
            &assets,
            &format!("css/{route}.css"),
            &format!(".{route} {{ padding: 4px; }}\n"),
        );
        fs::write(public.join(doc), format!(
            r#"<!doctype html><html><head><link rel="stylesheet" href="/assets/{common}"><link rel="stylesheet" href="/assets/{css}"><script type="importmap">{map}</script></head><body class="{route}"><script type="module" src="/assets/{main}"></script></body></html>"#,
        )).expect("write public document");
    }
}

fn write_hashed_asset(assets: &Path, logical: &str, body: &str) -> String {
    use sha2::{Digest, Sha256};
    let (stem, ext) = logical.rsplit_once('.').expect("asset extension");
    let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
    let name = format!("{stem}.{}.{ext}", &hash[..10]);
    fs::write(assets.join(&name), body).expect("write hashed asset");
    name
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
        if let Ok(res) = http
            .get(format!("http://127.0.0.1:{port}/boundary.html"))
            .send()
        {
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
/// document — the fixture supplies hashed files only, so these would 404.
fn assert_no_unhashed_refs(html: &str, context: &str) {
    for logical in [
        "main.js",
        "mapped.js",
        "chunks/widget.js",
        "chunks/value.js",
        "css/common.css",
        "css/root.css",
        "css/about.css",
    ] {
        assert!(
            !html.contains(&format!("/assets/{logical}")),
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

    for (doc, route) in [("boundary.html", "root"), ("about/boundary.html", "about")] {
        let html = http
            .get(format!("{base}/{doc}"))
            .send()
            .expect("GET document")
            .error_for_status()
            .expect("document must resolve")
            .text()
            .expect("read document");
        assert_no_unhashed_refs(&html, &context);
        let imports = inline_importmap(&html, &context);
        let main = imports["boundary/main"]
            .as_str()
            .expect("entry in import map");
        assert!(main.starts_with("/assets/main.") && main.ends_with(".js"));
        for stem in ["main", "css/common", &format!("css/{route}")] {
            assert!(hashed_name_present(&html, stem), "missing {stem}: {html}");
        }
        let refs = collect_live_refs(&http, &base, &html, &context);
        assert_surviving_refs(&refs, route);
        let map = http
            .get(format!("{base}/assets/importmap.json"))
            .send()
            .expect("GET importmap")
            .error_for_status()
            .expect("importmap must resolve")
            .bytes()
            .expect("read importmap");
        let disk: serde_json::Value = serde_json::from_slice(&map).expect("parse importmap");
        assert_eq!(
            &inline_importmap(&html, &context),
            disk["imports"].as_object().unwrap()
        );
    }
}

/// Normalize fixture hashes only to assert the expected logical closure.
/// Serve/build parity below compares the full URLs, including hashes.
fn strip_asset_hash(url: &str) -> String {
    let Some((base, ext)) = url
        .rsplit_once('.')
        .filter(|(_, ext)| matches!(*ext, "js" | "css"))
    else {
        return url.to_string();
    };
    let Some((stem, hash)) = base.rsplit_once('.') else {
        return url.to_string();
    };
    let is_hash = hash.len() == 10
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if is_hash {
        format!("{stem}.{ext}")
    } else {
        url.to_string()
    }
}

fn assert_surviving_refs(refs: &BTreeSet<String>, route: &str) {
    let expected = [
        "main.js",
        "mapped.js",
        "chunks/widget.js",
        "chunks/value.js",
        "css/common.css",
        &format!("css/{route}.css"),
    ]
    .into_iter()
    .map(|name| format!("/assets/{name}"))
    .collect();
    assert_eq!(
        strip_ref_set(refs),
        expected,
        "complete surviving asset closure"
    );
}

fn strip_ref_set(refs: &BTreeSet<String>) -> BTreeSet<String> {
    refs.iter().map(|url| strip_asset_hash(url)).collect()
}

#[test]
fn serve_and_build_resolve_identical_asset_urls() {
    // Public files are copied unchanged, so even the hashes must match.
    let serve = spawn_serve();
    let http = client();
    let base = format!("http://127.0.0.1:{}", serve.port);
    let live_context = format!("serve.log:\n{}", serve.log());
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
    for (doc, route) in [("boundary.html", "root"), ("about/boundary.html", "about")] {
        let served_html = http
            .get(format!("{base}/{doc}"))
            .send()
            .expect("GET document")
            .error_for_status()
            .expect("document must resolve")
            .text()
            .expect("read document");
        let dist_html = fs::read_to_string(dist_client.join(doc)).expect("read dist html");
        let live_refs = collect_live_refs(&http, &base, &served_html, &live_context);
        let disk_refs = collect_disk_refs(&dist_client, &dist_html);
        assert_surviving_refs(&live_refs, route);
        assert_surviving_refs(&disk_refs, route);
        assert_eq!(
            inline_importmap(&served_html, &live_context),
            inline_importmap(&dist_html, doc),
            "serve and build must inline the same import map for {doc}"
        );
        assert_eq!(
            live_refs, disk_refs,
            "serve and build must resolve identical asset URLs for {doc}\n{live_context}"
        );
    }
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
    for (doc, route) in [("boundary.html", "root"), ("about/boundary.html", "about")] {
        let html = fs::read_to_string(dist_client.join(doc)).expect("read dist html");
        assert_no_unhashed_refs(&html, doc);
        let refs = collect_disk_refs(&dist_client, &html);
        assert_surviving_refs(&refs, route);
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
    let index_html = fs::read_to_string(dist_client.join("boundary.html")).expect("read dist html");
    let inline = inline_importmap(&index_html, "dist boundary.html");
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
    let mut lock_packages = serde_json::Map::new();
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
        let integrity = compute_package_integrity(&module_dir).expect("package integrity");
        let package_name = format!("@deka/{package}");
        lock_packages.insert(
            package_name.clone(),
            serde_json::json!([
                format!("{package_name}@0.0.0"),
                format!("local:{package_name}"),
                {
                    "moduleGraph": { "hash": integrity.module_graph },
                    "fsGraph": { "hash": integrity.fs_graph }
                },
                ""
            ]),
        );
    }
    fs::write(
        dir.join("deka.lock"),
        serde_json::json!({
            "lockfileVersion": 1,
            "packages": lock_packages
        })
        .to_string(),
    )
    .expect("write deka.lock");
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
