//! deka#764 — the end-to-end gate for deka#743 (phase 4 of the
//! built-artifact-manifest-v2 track): a built artifact must serve with the
//! source tree, the compiler cache, and dsc ALL gone.
//!
//! Phases 1–3 can all be true while a hidden dependency on source or dsc
//! survives — one already did (deka#811, invisible because the cache filled
//! the gap). This gate is what makes the tracker's claim testable, so it is
//! deliberately adversarial about what "removed" means:
//!
//! - The source tree is not renamed or moved aside — after `deka build`, the
//!   whole project is deleted (`TempDir::drop`). The deploy directory is a
//!   fresh tree containing *only* `dist/`. Sources do not exist anywhere the
//!   test can reach, so a serve-time recompile has nothing to read.
//! - The page source is additionally mutated to a distinct marker before the
//!   deletion, so a serve that somehow reconstructed source would render the
//!   MUTATED marker and fail the BUILT-marker assertion.
//! - `.cache` is gone with the source tree, and the user-global cache
//!   (`~/.deka/cache` / `$XDG_CACHE_HOME/deka`, deka#765) is pointed at empty
//!   directories — no cache on the box can fill a gap the way #811's did.
//! - dsc: `DEKA_DSC` is removed from the serve environment and a poisoned
//!   canary named `dsc` is put FIRST on `PATH`. If serve execs `dsc` by any
//!   name lookup, the canary records the invocation and exits 42, which fails
//!   the serve outright; the test additionally asserts the canary's log was
//!   never written. dsc resolution is `DEKA_DSC` then `PATH`
//!   (`runtime_core::dsc::find_dsc`), so this covers every channel the
//!   runtime has for reaching the compiler.
//!
//! What this harness cannot remove, stated honestly: the `deka` binary
//! itself. It embeds the `ui/*` runtime sources (`deka_ui` crate) that the
//! loader materializes at serve time; those bytes travel inside the runtime,
//! not in source or dsc. The gate asserts the canary is untouched and the
//! routes serve, not that the binary contains no code paths at all.
//!
//! Known finding (reported on deka#764, owned by phase 3 / deka#763): in
//! this deploy form the loader's `materialize_ui_module`
//! (`crates/pool/src/esm_loader.rs`) writes the embedded `ui/*` sources into
//! `dist/server/.cache/dekascript/ui/` — a serve-time write *into* the
//! artifact. The bytes are not source or dsc output, so the gate passes, but
//! spec §5.2 classifies an unlisted file under `dist/server/` as tampered,
//! which the phase-3 loader's verification will trip over. This test
//! deliberately does not assert byte-identical `dist/` so the gate stays
//! green while that cleanup lands in its owning phase.

use reqwest::blocking::Client;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BUILT_MARKER: &str = "BUILT-MARKER-764";
const MUTATED_MARKER: &str = "MUTATED-MARKER-764";
const API_MARKER: &str = "API-MARKER-764";
const BUILD_VALUE_MARKER: &str = "hello from build 764";
const PUBLIC_MARKER: &str = "gate public file 764";

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

/// dsc for the build step only: explicit DEKA_DSC when set, else on PATH
/// (CI pins `DEKA_DSC=.ci/dsc` for `cargo test --workspace`). The serve
/// phase strips this — reaching for it there is what the gate detects.
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

/// The gate project: a static page, an island page (hashed client assets),
/// an API route, a request-time `build{}` route (resolves
/// `dist/server/.values/` per hit), and a public file. Every route class
/// deka#764 names is exercised against the gutted artifact.
fn write_gate_project(dir: &Path) {
    init_project(dir);
    fs::write(
        dir.join("app").join("page.dsx"),
        format!("export fn Page() {{\n  return <div>{BUILT_MARKER}</div>;\n}}\n"),
    )
    .expect("write page.dsx");
    fs::create_dir_all(dir.join("app").join("counter")).expect("mkdir counter route");
    fs::write(
        dir.join("app").join("counter").join("page.dsx"),
        concat!(
            "import { signal } from \"ui/reactive\"\n",
            "const pair = signal(1)\n",
            "const get = pair[0]\n",
            "const set = pair[1]\n",
            "\n",
            "export fn Counter() {\n",
            "    return <button onClick={fn() { set(get() + 1) }}>{get()}</button>\n",
            "}\n",
            "\n",
            "export fn Page() {\n",
            "    return <main><Counter client:load /></main>\n",
            "}\n",
        ),
    )
    .expect("write island page");
    fs::create_dir_all(dir.join("api").join("hello")).expect("mkdir api/hello");
    fs::write(
        dir.join("api").join("hello").join("route.ds"),
        format!(
            "interface Response {{\n  status: number,\n  body: string\n}}\n\
             export fn GET() Response {{\n  return {{ status: 200, body: \"{API_MARKER}\" }};\n}}\n"
        ),
    )
    .expect("write api route");
    fs::create_dir_all(dir.join("app").join("posts").join("[slug]")).expect("mkdir posts route");
    fs::write(
        dir.join("app")
            .join("posts")
            .join("[slug]")
            .join("page.dsx"),
        format!(
            "interface PageProps {{ slug: string }}\n\
             struct Post {{ title: string }}\n\
             export const prerender = false\n\
             const posts: Array<Post> = build {{\n\
             \x20   return Ok([Post{{title:\"{BUILD_VALUE_MARKER}\"}}])\n\
             }}\n\
             export fn Page(props: PageProps) {{\n\
             \x20   return <article><h1>{{posts[0].title}}</h1><p>{{props.slug}}</p></article>;\n\
             }}\n"
        ),
    )
    .expect("write build page");
    fs::write(
        dir.join("public").join("gate-probe.txt"),
        format!("{PUBLIC_MARKER}\n"),
    )
    .expect("write public file");
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

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("mkdir copy dest");
    for entry in fs::read_dir(src).expect("read copy src").flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy artifact file");
        }
    }
}

/// Every file under `dir`.
fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).expect("read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// The deploy tree must carry no compiler input: a `.ds`/`.dsx` anywhere
/// means the removal was weaker than the gate requires. (`dist/server/`
/// legitimately contains `deka.json`/`deka.lock` — spec §1: the policy the
/// artifact runs under travels with it — so the source-root marker check is
/// the separate assertion on the deploy root, not this walk.)
fn assert_no_compiler_input(root: &Path) {
    for file in walk_files(root) {
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        assert!(
            !matches!(ext, "ds" | "dsx"),
            "deploy tree must not contain compiler input: {}",
            file.display()
        );
    }
}

/// Additionally, before serve, no compiler cache may exist in the deploy
/// tree at all. (After serve the loader currently materializes its embedded
/// `ui/*` sources under `dist/server/.cache/` — the finding documented in
/// the module header — so this runs pre-serve only.)
fn assert_no_cache_dirs(root: &Path) {
    for file in walk_files(root) {
        assert!(
            !file
                .components()
                .any(|c| c.as_os_str() == std::ffi::OsStr::new(".cache")),
            "deploy tree must not contain a compiler cache: {}",
            file.display()
        );
    }
}

/// A poisoned canary named `dsc`: records every invocation and fails. Put
/// first on the serve `PATH`; if the artifact path execs the compiler by
/// name, the serve dies and the log proves the reach.
fn write_canary_dsc(canary_dir: &Path, log_path: &Path) {
    fs::create_dir_all(canary_dir).expect("mkdir canary dir");
    let script = format!(
        "#!/bin/sh\necho \"dsc invoked: $@\" >> '{}'\nexit 42\n",
        log_path.display()
    );
    let dsc = canary_dir.join("dsc");
    fs::write(&dsc, script).expect("write canary dsc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dsc).expect("canary metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dsc, perms).expect("chmod canary");
    }
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

/// Every `/assets/...` URL referenced by an HTML body, unique, in order.
fn asset_refs(html: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut cursor = 0;
    while let Some(found) = html[cursor..].find("/assets/") {
        let start = cursor + found;
        let rest = &html[start..];
        let end = rest.find(['"', '\'', ' ', '>', '?']).unwrap_or(rest.len());
        let url = rest[..end].to_string();
        if !refs.contains(&url) {
            refs.push(url);
        }
        cursor = start + end;
    }
    refs
}

#[test]
fn artifact_only_gate_serves_real_routes_without_source_cache_or_dsc() {
    // Build the gate project in a temp tree with the suite's pinned dsc.
    let project = tempfile::tempdir().expect("create temp project dir");
    write_gate_project(project.path());
    let (success, combined) = run_build(project.path());
    assert!(success, "deka build failed: {combined}");

    // The build published a self-describing v2 artifact.
    let dist = project.path().join("dist");
    let manifest_path = dist.join("build-manifest.json");
    assert!(manifest_path.is_file(), "dist/build-manifest.json missing");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    assert_eq!(
        manifest.get("format").and_then(|v| v.as_str()),
        Some("deka.artifact@2"),
        "dist manifest must be artifact v2: {manifest}"
    );
    assert!(
        dist.join("build-manifest.sha256").is_file(),
        "manifest sidecar missing"
    );
    assert!(
        dist.join("server").join("serve-entry.js").is_file(),
        "dist/server/serve-entry.js missing"
    );
    assert!(
        dist.join("server").join(".values").is_dir(),
        "dist/server/.values missing (request-time build values ship in the artifact)"
    );
    for file in walk_files(&dist) {
        let bytes = fs::read(&file).expect("read dist file");
        assert!(
            !bytes
                .windows(b"deka:dev/".len())
                .any(|window| window == b"deka:dev/"),
            "dist/ must not reference deka:dev/ specifiers: {}",
            file.display()
        );
    }

    // Mutate the page AFTER the build: a serve that recompiled anything would
    // render MUTATED_MARKER instead of the built bytes.
    fs::write(
        project.path().join("app").join("page.dsx"),
        format!("export fn Page() {{\n  return <div>{MUTATED_MARKER}</div>;\n}}\n"),
    )
    .expect("mutate page.dsx");

    // The deploy: a fresh tree holding ONLY dist/. Then the source project is
    // deleted outright — not renamed, not moved aside. Together with the
    // empty user-cache dirs below, no source, cache, or dsc exists anywhere
    // the serve can reach.
    let deploy = tempfile::tempdir().expect("create deploy dir");
    copy_dir(&dist, &deploy.path().join("dist"));
    drop(project);

    let root = deploy.path();
    assert_no_compiler_input(root);
    assert_no_cache_dirs(root);
    assert!(
        !root.join("deka.json").exists() && !root.join("app").exists(),
        "deploy tree must not contain the project source at all"
    );

    // Poisoned canary first on PATH; DEKA_DSC stripped; user-global caches
    // pointed at empty directories so nothing can fill a gap (#811).
    let canary_dir = root.join("canary-bin");
    let canary_log = root.join("canary-dsc-invocations.log");
    write_canary_dsc(&canary_dir, &canary_log);
    let empty_home = canary_dir.join("empty-home");
    let empty_xdg = canary_dir.join("empty-xdg");
    fs::create_dir_all(&empty_home).expect("mkdir empty home");
    fs::create_dir_all(&empty_xdg).expect("mkdir empty xdg");

    let port = free_port();
    let log_path = root.join(format!("serve-gate-{port}.log"));
    let log = fs::File::create(&log_path).expect("serve log");
    let path_with_canary = format!(
        "{}:{}",
        canary_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root)
        .env_remove("DEKA_DSC")
        .env("PATH", path_with_canary)
        .env("HOME", &empty_home)
        .env("XDG_CACHE_HOME", &empty_xdg)
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka serve");
    let serve = KillOnDrop(Some(child));
    wait_ready(port, &log_path);
    let log_text = || fs::read_to_string(&log_path).unwrap_or_default();

    let http = client();
    let base = format!("http://127.0.0.1:{port}");

    // Page: the BUILT bytes, never the mutated source.
    let body = http
        .get(format!("{base}/"))
        .send()
        .expect("GET /")
        .text()
        .expect("home html");
    assert!(
        body.contains(BUILT_MARKER),
        "the artifact-only gate must render the BUILT marker; got:\n{body}\nserve log:\n{}",
        log_text()
    );
    assert!(
        !body.contains(MUTATED_MARKER),
        "the MUTATED source marker leaked in — serve reconstructed source: {body}"
    );

    // Island page: SSR output plus the hashed client assets, all served from
    // the artifact's client root.
    let island = http
        .get(format!("{base}/counter"))
        .send()
        .expect("GET /counter")
        .text()
        .expect("island html");
    assert!(
        island.contains("deka-island"),
        "the island page must SSR its island marker; got:\n{island}\nserve log:\n{}",
        log_text()
    );
    let refs = asset_refs(&island);
    assert!(
        !refs.is_empty(),
        "the island page must reference hashed /assets/ client chunks; got:\n{island}"
    );
    for url in &refs {
        let res = http
            .get(format!("{base}{url}"))
            .send()
            .unwrap_or_else(|err| panic!("GET {url} failed: {err}\nserve log:\n{}", log_text()));
        assert_eq!(
            res.status().as_u16(),
            200,
            "asset {url} must serve 200 from the artifact; serve log:\n{}",
            log_text()
        );
        let bytes = res.bytes().expect("asset bytes");
        assert!(
            bytes.len() > 64,
            "asset {url} must carry real payload, got {} bytes",
            bytes.len()
        );
    }

    // API route from the built server tree.
    let api = http
        .get(format!("{base}/api/hello"))
        .send()
        .expect("GET /api/hello")
        .text()
        .expect("api body");
    assert!(
        api.contains(API_MARKER),
        "built api entry must answer /api/hello: {api}\nserve log:\n{}",
        log_text()
    );

    // Request-time route: the build{} value resolves from dist/server/.values
    // on every hit — with the compiler cache gone, a cache lookup would 500.
    let post = http
        .get(format!("{base}/posts/hello"))
        .send()
        .expect("GET /posts/hello")
        .text()
        .expect("post html");
    assert!(
        post.contains(BUILD_VALUE_MARKER),
        "request-time build value must resolve from the artifact: {post}\nserve log:\n{}",
        log_text()
    );

    // Public file from the artifact's client root.
    let public = http
        .get(format!("{base}/gate-probe.txt"))
        .send()
        .expect("GET /gate-probe.txt")
        .text()
        .expect("public body");
    assert!(
        public.contains(PUBLIC_MARKER),
        "public file must serve from the artifact: {public}"
    );

    drop(serve);

    // The gate: serve never reached for the compiler...
    assert!(
        !canary_log.exists(),
        "serve reached for dsc — canary was invoked:\n{}",
        fs::read_to_string(&canary_log).unwrap_or_default()
    );
    // ...and never materialized source or compiler input into the deploy.
    assert_no_compiler_input(root);
    let entries: Vec<String> = fs::read_dir(root)
        .expect("read deploy root")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let mut unexpected: Vec<&String> = entries
        .iter()
        .filter(|name| {
            name.as_str() != "dist"
                && name.as_str() != "canary-bin"
                && !name.starts_with("serve-gate-")
        })
        .collect();
    unexpected.sort();
    assert!(
        unexpected.is_empty(),
        "serve must not write anything but its log into the deploy tree: {unexpected:?}"
    );
}
