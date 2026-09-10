//! deka#762: the point of deka#743 is that "the emitted entry is loaded" and
//! "the source is recompiled and happens to match" are two DIFFERENT
//! postures, and only the first is production-correct. These tests make the
//! two postures observably different: build the project, then mutate the
//! source (and delete it, and delete the compile cache) before serving.
//!
//! - If `deka serve` loads the built artifact (`dist/server/serve-entry.js`,
//!   per the manifest v2 layout), the response carries the BUILT marker.
//! - If `deka serve` recompiled the (mutated) source — the old behavior —
//!   the response carries the MUTATED marker or fails outright.
//!
//! A test that cannot tell these apart proves nothing; these can.

use reqwest::blocking::Client;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BUILT_MARKER: &str = "BUILT-MARKER-1";
const MUTATED_MARKER: &str = "MUTATED-MARKER-2";
const ARTIFACT_MARKER: &str = "ARTIFACT-MARKER-3";
const API_MARKER: &str = "API-MARKER-1";

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

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

fn run_build(dir: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(dir)
        .env("DEKA_DSC", dsc_path())
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// dsc for the build step: explicit DEKA_DSC when set, else on PATH (CI).
fn dsc_path() -> String {
    std::env::var("DEKA_DSC").unwrap_or_else(|_| "dsc".to_string())
}

fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client")
}

fn wait_ready(port: u16, log_path: &Path) {
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
        "deka serve did not become ready on port {port}\nserve log:\n{}",
        fs::read_to_string(log_path).unwrap_or_default()
    );
}

fn start_serve(dir: &Path, port: u16) -> (KillOnDrop, PathBuf) {
    let log_path = dir.join(format!("serve-{port}.log"));
    let log = fs::File::create(&log_path).expect("serve log");
    let child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(dir)
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka serve");
    (KillOnDrop(Some(child)), log_path)
}

/// Build a project, then sabotage every source-posture escape hatch:
/// mutate the page source, delete the app/ tree, and delete the compile
/// cache (which held the old serve-entry.dsx the serve path used to
/// recompile from).
fn built_then_sabotaged_project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        format!(
            "export const prerender = false\nexport fn Page() {{\n  return <div>{BUILT_MARKER}</div>;\n}}\n"
        ),
    )
    .expect("write page.dsx");
    fs::create_dir_all(project.path().join("api").join("hello")).expect("mkdir api/hello");
    fs::write(
        project
            .path()
            .join("api")
            .join("hello")
            .join("route.ds"),
        format!("interface Response {{\n  status: number,\n  body: string\n}}\nexport fn GET() Response {{\n  return {{ status: 200, body: \"{API_MARKER}\" }};\n}}\n"),
    )
    .expect("write api/hello/route.ds");

    let (success, combined) = run_build(project.path());
    assert!(success, "deka build failed: {combined}");

    // The build published the artifact manifest v2 + server entries.
    let manifest_path = project.path().join("dist").join("build-manifest.json");
    assert!(manifest_path.is_file(), "dist/build-manifest.json missing");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    assert_eq!(
        manifest.get("format").and_then(|v| v.as_str()),
        Some("deka.artifact@2"),
        "dist manifest must be artifact v2: {manifest}"
    );
    let entries = manifest
        .pointer("/server/entries")
        .and_then(|v| v.as_array())
        .expect("server.entries array");
    assert!(
        entries.iter().any(|e| {
            e.get("id").and_then(|v| v.as_str()) == Some("page:/")
                && e.get("module").and_then(|v| v.as_str()).map(str::is_empty) == Some(false)
        }),
        "manifest must describe the / page entry with a module: {manifest}"
    );
    assert!(
        project
            .path()
            .join("dist")
            .join("build-manifest.sha256")
            .is_file(),
        "manifest sidecar missing next to {manifest_path:?}"
    );
    assert!(
        project
            .path()
            .join("dist")
            .join("server")
            .join("serve-entry.js")
            .is_file(),
        "dist/server/serve-entry.js missing"
    );

    // Sabotage: any serve-time recompile now produces something observably
    // different from the built artifact.
    fs::write(
        project.path().join("app").join("page.dsx"),
        format!(
            "export const prerender = false\nexport fn Page() {{\n  return <div>{MUTATED_MARKER}</div>;\n}}\n"
        ),
    )
    .expect("mutate page.dsx");
    fs::remove_dir_all(project.path().join("app")).expect("delete app/");
    fs::remove_dir_all(project.path().join(".cache")).expect("delete .cache/");

    project
}

/// Change the actual server artifact, then publish a matching descriptor.
/// This is intentionally different from tampering: a digest mismatch must
/// refuse to boot. Re-anchoring this controlled edit makes it a new valid
/// artifact, so the response can prove which bytes the loader executed.
fn mutate_built_server_artifact(project: &Path) {
    let dist = project.join("dist");
    let server = dist.join("server");
    let mut js_files = Vec::new();
    collect_js_files(&server, &mut js_files);
    let page = js_files
        .into_iter()
        .find(|path| {
            fs::read_to_string(path)
                .map(|source| source.contains(BUILT_MARKER))
                .unwrap_or(false)
        })
        .expect("built server graph must contain the page marker");
    let original = fs::read_to_string(&page).expect("read built page module");
    fs::write(&page, original.replace(BUILT_MARKER, ARTIFACT_MARKER))
        .expect("mutate built page module");

    let mut manifest = runtime_core::framework::ArtifactManifestV2::load_verified(&dist)
        .expect("load original artifact manifest");
    manifest
        .record_payloads(&dist)
        .expect("rehash changed artifact");
    manifest
        .write_into(&dist)
        .expect("write changed artifact descriptor");
}

fn collect_js_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read artifact server tree") {
        let path = entry.expect("server entry").path();
        if path.is_dir() {
            collect_js_files(&path, out);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("js") {
            out.push(path);
        }
    }
}

#[test]
fn serve_loads_the_built_entry_not_a_recompile() {
    let project = built_then_sabotaged_project();
    let port = free_port();
    let (_child, log_path) = start_serve(project.path(), port);
    wait_ready(port, &log_path);

    let http = client();
    let base = format!("http://127.0.0.1:{port}");
    let body = http
        .get(format!("{base}/"))
        .send()
        .expect("GET /")
        .text()
        .expect("home html");

    assert!(
        body.contains(BUILT_MARKER),
        "serving the built artifact must render the BUILT marker; got:\n{body}\nserve log:\n{}",
        fs::read_to_string(&log_path).unwrap_or_default()
    );
    assert!(
        !body.contains(MUTATED_MARKER),
        "the MUTATED source marker leaked in — serve recompiled source instead of \
         loading dist/server: {body}"
    );

    // The api route is served from the built server tree too.
    let api = http
        .get(format!("{base}/api/hello"))
        .send()
        .expect("GET /api/hello")
        .text()
        .expect("api body");
    assert!(
        api.contains(API_MARKER),
        "built api entry must answer /api/hello: {api}"
    );
}

#[test]
fn serve_dist_directory_form_loads_the_built_entry() {
    let project = built_then_sabotaged_project();
    let dist = project.path().join("dist");
    let port = free_port();
    let log_path = project.path().join(format!("serve-dist-{port}.log"));
    let log = fs::File::create(&log_path).expect("serve log");
    let child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(&dist)
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka serve dist");
    let _child = KillOnDrop(Some(child));
    wait_ready(port, &log_path);

    let body = client()
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .expect("GET /")
        .text()
        .expect("home html");
    assert!(
        body.contains(BUILT_MARKER) && !body.contains(MUTATED_MARKER),
        "`deka serve dist/` must load the built entry, not recompile: {body}"
    );
}

#[test]
fn serve_executes_a_reanchored_mutation_of_the_built_artifact() {
    let project = built_then_sabotaged_project();
    mutate_built_server_artifact(project.path());
    let port = free_port();
    let (_child, log_path) = start_serve(project.path(), port);
    wait_ready(port, &log_path);

    let body = client()
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .expect("GET /")
        .text()
        .expect("response body");
    assert!(
        body.contains(ARTIFACT_MARKER),
        "the response must come from the mutated dist/server module, not a source recompile: {body}\nserve log:\n{}",
        fs::read_to_string(&log_path).unwrap_or_default()
    );
    assert!(
        !body.contains(BUILT_MARKER) && !body.contains(MUTATED_MARKER),
        "served bytes did not come exclusively from the changed artifact: {body}"
    );
}
