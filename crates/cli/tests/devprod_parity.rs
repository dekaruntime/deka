//! RFD 24 §11.2 — dev/prod module-boundary conformance.
//!
//! Dev (`deka serve --dev`) and production (`deka build`) must be two shapes
//! of ONE compiler: for the same module, dev and prod emission must agree
//! module-for-module — same module boundaries, same import graph, same
//! elimination decisions — differing only in how modules are grouped for
//! delivery. This is the conformance test the RFD calls for, motivated by
//! four "two artefacts that must agree" bugs shipped in one week
//! (deka#582, deka#583, website#117, website#129).
//!
//! The fixture project has a small multi-module app:
//!
//! ```text
//! app/layout.dsx, app/page.dsx, app/about/page.dsx   (route modules)
//! lib/shared.ds  -> imports lib/leaf.ds              (shared non-route module)
//! lib/leaf.ds                                        (leaf of the graph)
//! lib/unused.ds                                      (never imported: unreachable)
//! lib/orphan.ds                                      (imported by page.dsx but
//!                                                     never used: shaken)
//! ```
//!
//! Dev side: a real `deka serve --dev` is started and both routes are
//! fetched. Production side: `deka build`. Both compile the same generated
//! entry (`.cache/dekascript/serve-entry.dsx`) through the same module
//! graph compiler; the test pins that they agree:
//!
//! 1. Entry parity — both modes generate byte-identical serve-entry.dsx,
//!    so both compile the exact same module graph rooted at the same module.
//! 2. Module/edge parity — every kept module's body marker appears in both
//!    modes' rendered output; the `page -> shared -> leaf` import chain is
//!    proven by a composite marker that can only render if both edges were
//!    compiled in that mode.
//! 3. Elimination parity — the shaken-out modules (`unused.ds`, and
//!    `orphan.ds` whose import is never used and is dropped by graph
//!    shaking) are absent from BOTH modes' emitted artifacts.

use reqwest::blocking::Client;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Renders as "shared-body-marker leaf-body-marker"; can only appear if the
/// compiled import chain page -> shared -> leaf survived in that mode.
const COMPOSITE_MARKER: &str = "shared-body-marker leaf-body-marker";
const ORPHAN_MARKER: &str = "orphan-body-marker";
const UNUSED_MARKER: &str = "unused-body-marker";

struct KillOnDrop(Option<std::process::Child>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral")
        .local_addr()
        .expect("local addr")
        .port()
}

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
}

/// Scaffolds the multi-module fixture into `dir` (via `deka init`, like the
/// other cli integration tests) and returns the project root.
fn write_fixture(dir: &Path) {
    init_project(dir);

    // Shared non-route modules. shared.ds imports leaf.ds; unused.ds is
    // never imported; orphan.ds is imported by page.dsx but never used, so
    // graph shaking must drop it.
    fs::create_dir_all(dir.join("lib")).expect("mkdir lib");
    fs::write(
        dir.join("lib").join("leaf.ds"),
        "export fn leaf_marker() string {\n    return \"leaf-body-marker\";\n}\n",
    )
    .expect("write leaf.ds");
    fs::write(
        dir.join("lib").join("shared.ds"),
        "import { leaf_marker } from \"./leaf.ds\"\nexport fn shared_message() string {\n    return \"shared-body-marker \" + leaf_marker();\n}\n",
    )
    .expect("write shared.ds");
    fs::write(
        dir.join("lib").join("unused.ds"),
        "export fn unused_secret() string {\n    return \"unused-body-marker\";\n}\n",
    )
    .expect("write unused.ds");
    fs::write(
        dir.join("lib").join("orphan.ds"),
        "export fn orphan_secret() string {\n    return \"orphan-body-marker\";\n}\n",
    )
    .expect("write orphan.ds");

    // Both routes import the shared module; the home page also imports (but
    // never uses) the orphan module.
    fs::write(
        dir.join("app").join("page.dsx"),
        "import { shared_message } from \"../lib/shared.ds\"\nimport { orphan_secret } from \"../lib/orphan.ds\"\nexport fn Page() {\n    return <main><h1>home {shared_message()}</h1></main>;\n}\n",
    )
    .expect("write app/page.dsx");
    let about = dir.join("app").join("about");
    fs::create_dir_all(&about).expect("mkdir app/about");
    fs::write(
        about.join("page.dsx"),
        "import { shared_message } from \"../../lib/shared.ds\"\nexport fn Page() {\n    return <main><h1>about {shared_message()}</h1></main>;\n}\n",
    )
    .expect("write app/about/page.dsx");
}

fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client")
}

fn wait_ready(port: u16) {
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
    panic!("deka serve did not become ready on port {port}");
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

fn serve_entry(root: &Path) -> PathBuf {
    root.join(".cache")
        .join("dekascript")
        .join("serve-entry.dsx")
}

fn read_serve_entry(root: &Path) -> String {
    fs::read_to_string(serve_entry(root)).expect("read generated serve-entry.dsx")
}

/// Collect every file under `dir`, joined with newline, for diagnostics.
fn tree_listing(dir: &Path) -> String {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn dev_and_prod_emit_the_same_modules() {
    let project = tempfile::tempdir().expect("create temp project dir");
    write_fixture(project.path());

    // --- Dev side: a real `deka serve --dev` compiles the app through the
    // module graph and serves unbundled native ESM. ---
    let port = free_port();
    let log_path = project.path().join("serve.log");
    let log = fs::File::create(&log_path).expect("serve.log");
    let child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(project.path())
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka serve");
    let mut child = KillOnDrop(Some(child));
    let serve_log = || fs::read_to_string(&log_path).unwrap_or_default();
    wait_ready(port);
    let http = client();
    let base = format!("http://127.0.0.1:{port}");

    let dev_home = http
        .get(format!("{base}/"))
        .send()
        .expect("GET /")
        .text()
        .expect("dev home html");
    let dev_about = http
        .get(format!("{base}/about"))
        .send()
        .expect("GET /about")
        .text()
        .expect("dev about html");

    // The import chain page -> shared -> leaf must have been compiled in dev:
    // the composite marker can only render if both edges survived.
    for (route, body) in [("/", &dev_home), ("/about", &dev_about)] {
        assert!(
            body.contains(COMPOSITE_MARKER),
            "dev render of {route} must include the shared->leaf chain: {body}\nserve.log:\n{}",
            serve_log()
        );
    }
    // Shaken modules must not leak into dev emission, even when fetched.
    let dev_bogus = http
        .get(format!("{base}/no-such-route"))
        .send()
        .expect("GET /no-such-route")
        .text()
        .expect("dev 404 html");
    for (name, body) in [
        ("/", dev_home.as_str()),
        ("/about", dev_about.as_str()),
        ("/no-such-route", dev_bogus.as_str()),
    ] {
        assert!(
            !body.contains(ORPHAN_MARKER),
            "orphan.ds is imported but never used; graph shaking must drop it, \
             yet its marker leaked into dev render of {name}"
        );
        assert!(
            !body.contains(UNUSED_MARKER),
            "unused.ds is never imported; its marker must not appear in dev \
             render of {name}"
        );
    }

    // The serve path's entry generation, captured before build overwrites it.
    let dev_entry = read_serve_entry(project.path());
    assert!(
        dev_entry.contains("app/about/page.dsx") && dev_entry.contains("app/page.dsx"),
        "generated serve-entry should wire both routes: {dev_entry}"
    );

    if let Some(mut serve_child) = child.0.take() {
        let _ = serve_child.kill();
        let _ = serve_child.wait();
    }

    // --- Production side: `deka build` runs the same graph compiler and
    // bundles/prerenders for the worker. ---
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build must succeed on a project `deka serve --dev` compiles \
         (RFD 24 §11.2 parity): {combined}"
    );

    // 1. Entry parity: both modes generate byte-identical serve-entry.dsx,
    //    so both compile the same module graph from the same root module.
    let prod_entry = read_serve_entry(project.path());
    assert_eq!(
        dev_entry, prod_entry,
        "serve-entry.dsx must be generated identically by `deka serve --dev` \
         and `deka build`; the two modes drifted in entry generation.\n\
         --- dev ---\n{dev_entry}\n--- prod ---\n{prod_entry}"
    );

    // 2. Module/edge parity: the prerendered dist HTML went through the same
    //    graph compile, so the same import chain must be present.
    let dist_home = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read dist/client/index.html");
    let dist_about = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("about")
            .join("index.html"),
    )
    .expect("read dist/client/about/index.html");
    for (route, body) in [("/", &dist_home), ("/about", &dist_about)] {
        assert!(
            body.contains(COMPOSITE_MARKER),
            "prod prerender of {route} must include the shared->leaf chain, \
             exactly as dev did: {body}"
        );
        assert!(
            !body.contains("<!--deka-app-->"),
            "prod prerender of {route} must fill the app hole: {body}"
        );
    }

    // 3. Elimination parity: the shaken-out modules are absent from every
    //    emitted artifact, not just the HTML (this scans dist/ wholesale).
    let dist = project.path().join("dist");
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![dist.clone()];
    while let Some(current) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap_or_default();
                if text.contains(ORPHAN_MARKER) || text.contains(UNUSED_MARKER) {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "shaken-out modules (orphan.ds, unused.ds) leaked into prod emission: {offenders:?}\n{}",
        tree_listing(&dist)
    );

    // Module-boundary parity for the route tree: dist/server/app receives the
    // app's modules one-for-one — same boundaries, same paths.
    let dist_app = dist.join("server").join("app");
    for rel in ["layout.dsx", "page.dsx", "about/page.dsx", "not-found.dsx"] {
        assert!(
            dist_app.join(rel).is_file(),
            "dist/server/app must contain the app module {rel}:\n{}",
            tree_listing(&dist_app)
        );
    }
}

/// Fail-closed parity (deka#5, preserved by the §11.2 fix): `deka build`
/// must still reject genuinely invalid source under app/ — files with no
/// imports validate standalone; files with imports validate through the
/// module graph. Either way the diagnostic names the offending file and no
/// dist/ output is written.
#[test]
fn build_still_fails_closed_on_invalid_source() {
    let project = tempfile::tempdir().expect("create temp project dir");
    write_fixture(project.path());

    // Import-free broken file: standalone validation path.
    fs::write(
        project.path().join("app").join("broken.dsx"),
        "export function f(): int { return\n",
    )
    .expect("write broken.dsx");
    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must exit non-zero on invalid source under app/: {combined}"
    );
    assert!(
        combined.contains("broken.dsx"),
        "build failure diagnostic should name the offending file: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "deka build should not write dist/ output when validation fails"
    );
}

#[test]
fn build_still_fails_closed_on_invalid_importing_source() {
    let project = tempfile::tempdir().expect("create temp project dir");
    write_fixture(project.path());

    // Broken file WITH imports: module-graph validation path — the same
    // graph compilation `deka serve --dev` performs.
    fs::write(
        project
            .path()
            .join("app")
            .join("broken-import.dsx"),
        "import { shared_message } from \"../lib/shared.ds\"\nexport fn Broken() {\n    return <main>{shared_message()}</main>;\n",
    )
    .expect("write broken-import.dsx");
    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must exit non-zero on a graph-invalid importing source: {combined}"
    );
    assert!(
        combined.contains("broken-import.dsx"),
        "build failure diagnostic should name the offending file: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "deka build should not write dist/ output when validation fails"
    );
}
