//! deka#725: phase-aware permissions for `build {}` execution and recorded
//! local build inputs for targeted `deka dev` invalidation.
//!
//! The build phase grants no capability of its own (rfd#48): it executes
//! under the project's existing security policy, denied bridge calls surface
//! as source-linked permission diagnostics, permitted local reads are
//! recorded per slot (successful reads, absent paths, directory listings),
//! and `deka dev` rematerializes exactly the slots a local change affects.
//! Real dsc required (same as the other build suites).

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Scaffolds a fresh web project into `dir` via `deka init`.
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
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn run_with_args(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(cli_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run deka");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn manifest_path(project: &Path) -> PathBuf {
    project
        .join(".cache")
        .join("dekascript")
        .join("build-manifest.json")
}

fn manifest_json(project: &Path) -> serde_json::Value {
    serde_json::from_str(
        &fs::read_to_string(manifest_path(project)).expect("read build manifest"),
    )
    .expect("parse build manifest")
}

/// Writes deka.json with a security section; keeps the scaffold's shape.
fn write_security(project: &Path, security: &str) {
    fs::write(
        project.join("deka.json"),
        format!(
            "{{\n  \"name\": \"phase\",\n  \"type\": \"serve\",\n  \"serve\": {{ \"mode\": \"ds\" }},\n  \"security\": {security}\n}}\n"
        ),
    )
    .expect("write deka.json");
}

/// A [slug] page whose build body reads `path` and propagates failures: the
/// `build { ... }` expression starts on line 10 of this fixture.
fn slug_page_probing(path: &str) -> String {
    format!(
        "interface PageProps {{ slug: string }}\nstruct PostParam {{ slug: string }}\nasync fn probe(p: string) Promise<Result<Array<PostParam>, string>> {{\n  const res = await bridge fs.read_file(p)\n  return match (res) {{\n    Ok(v) => Ok([PostParam {{ slug: \"hello\" }}]),\n    Err(e) => Err(e)\n  }}\n}}\nexport const staticParams: Array<PostParam> = build {{\n  return await probe(\"{path}\")\n}}\nexport fn Page(props: PageProps) {{\n  return <article><h1>{{props.slug}}</h1></article>;\n}}\n"
    )
}

fn write_slug_page(project: &Path, source: &str) {
    let slug_dir = project.join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    fs::write(slug_dir.join("page.dsx"), source).expect("write slug page");
}

#[test]
fn denied_fs_read_in_build_fails_with_source_linked_diagnostic() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_security(project.path(), r#"{"prompt": false}"#);
    write_slug_page(project.path(), &slug_page_probing("data/x.json"));

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must fail when the build body reads a file the policy denies: {combined}"
    );
    assert!(
        combined.contains("SECURITY_CAPABILITY_DENIED"),
        "failure must carry the permission diagnostic: {combined}"
    );
    assert!(
        combined.contains("capability=read"),
        "diagnostic must name the read capability: {combined}"
    );
    assert!(
        combined.contains("page.dsx:10:"),
        "diagnostic must link the build block's source location: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "a failed build must not publish dist/: {combined}"
    );
}

#[test]
fn permitted_fs_reads_are_recorded_in_the_manifest() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_security(
        project.path(),
        r#"{"allow": {"read": ["./data"]}, "prompt": false}"#,
    );
    let slug_dir = project.path().join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    // One successful read, one directory listing, one absent path.
    fs::write(
        slug_dir.join("page.dsx"),
        "interface PageProps { slug: string }\nstruct PostParam { slug: string }\nexport const staticParams: Array<PostParam> = build {\n  const present = await bridge fs.read_file(\"data/slugs/a.txt\")\n  const listed = await bridge fs.read_dir(\"data\")\n  const missing = await bridge fs.read_file(\"data/missing.txt\")\n  return Ok([PostParam { slug: \"hello\" }])\n}\nexport fn Page(props: PageProps) {\n  return <article><h1>{props.slug}</h1></article>;\n}\n",
    )
    .expect("write slug page");
    fs::create_dir_all(project.path().join("data").join("slugs")).expect("mkdir data/slugs");
    fs::write(project.path().join("data").join("slugs").join("a.txt"), "a").expect("write data");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed when the policy permits the reads: {combined}"
    );

    let manifest = manifest_json(project.path());
    let slots = manifest["slots"].as_array().expect("slots array");
    assert_eq!(slots.len(), 1, "one build slot expected: {manifest}");
    let observations = slots[0]["observations"].as_array().expect("observations");
    let rendered: Vec<String> = observations
        .iter()
        .map(|observation| {
            format!(
                "{} {}",
                observation["kind"].as_str().expect("kind"),
                observation["path"].as_str().expect("path")
            )
        })
        .collect();
    assert_eq!(
        rendered,
        vec![
            "directory_listing data",
            "absent data/missing.txt",
            "read data/slugs/a.txt",
        ],
        "manifest must record the listing, the absent path, and the read: {manifest}"
    );
}

#[test]
fn network_and_db_inputs_record_no_filesystem_observations() {
    // A bridge call the policy denies fails before any host I/O happens; the
    // manifest must not fabricate filesystem observations for such inputs.
    // (Net/db refresh stays explicit/polling/webhook — rfd#48.)
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_security(project.path(), r#"{"prompt": false}"#);
    write_slug_page(project.path(), &slug_page_probing("data/x.json"));

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must fail when the policy denies the build body's read: {combined}"
    );
    assert!(
        combined.contains("SECURITY_CAPABILITY_DENIED"),
        "failure must carry the permission diagnostic: {combined}"
    );
    assert!(
        !manifest_path(project.path()).exists(),
        "a failed build publishes no manifest at all, so denied inputs are never \
         recorded as fabricated filesystem observations"
    );
}

#[test]
fn denied_capability_is_not_granted_to_runtime_code() {
    // Parity (rfd#48): the build phase widens nothing. The same fs read that
    // the build body cannot perform is equally denied to ordinary runtime
    // code under the same policy — and remains available to neither by
    // default, while an explicit allow unlocks both phases.
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_security(project.path(), r#"{"prompt": false}"#);
    fs::write(
        project.path().join("main.ds"),
        "async fn go() Promise<string> {\n  const res = await bridge fs.read_file(\"data/x.json\")\n  return match (res) {\n    Ok(v) => \"runtime-read-ok\",\n    Err(e) => \"runtime-read-denied\"\n  }\n}\nasync fn main() Promise<string> {\n  const r = await go()\n  unsafe { console.log(r) }\n  return r\n}\nmain()\n",
    )
    .expect("write main.ds");

    let (_, combined) = run_with_args(project.path(), &["run", "main.ds", "--no-prompt"]);
    assert!(
        combined.contains("runtime-read-denied"),
        "runtime fs read must be denied under the default policy: {combined}"
    );
    assert!(
        !combined.contains("runtime-read-ok"),
        "runtime fs read must not succeed under the default policy: {combined}"
    );
}

#[test]
fn explicit_allow_unlocks_both_phases_equally() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_security(
        project.path(),
        r#"{"allow": {"read": ["./data"]}, "prompt": false}"#,
    );
    fs::create_dir_all(project.path().join("data")).expect("mkdir data");
    fs::write(project.path().join("data").join("x.json"), "{}").expect("write data");
    fs::write(
        project.path().join("main.ds"),
        "async fn go() Promise<string> {\n  const res = await bridge fs.read_file(\"data/x.json\")\n  return match (res) {\n    Ok(v) => \"runtime-read-ok\",\n    Err(e) => \"runtime-read-denied\"\n  }\n}\nasync fn main() Promise<string> {\n  const r = await go()\n  unsafe { console.log(r) }\n  return r\n}\nmain()\n",
    )
    .expect("write main.ds");

    let (_, combined) = run_with_args(project.path(), &["run", "main.ds", "--no-prompt"]);
    assert!(
        combined.contains("runtime-read-ok"),
        "an explicit read allow must unlock runtime reads just as it unlocks build reads: {combined}"
    );

    write_slug_page(project.path(), &slug_page_probing("data/x.json"));
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed under the same explicit allow: {combined}"
    );
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

/// A [slug] page whose build body derives a marker from both a directory
/// listing and a file read: `<readdir count>/<picked.txt byte length>`.
/// Adding/removing files changes the first component; editing the picked
/// file changes the second. Everything is inlined because dsc plan emission
/// omits helpers referenced only inside `unsafe` arrows.
fn slug_page_count_over_len() -> &'static str {
    "interface PageProps { slug: string }\nstruct PostParam { slug: string }\nasync fn slugs() Promise<Result<Array<PostParam>, string>> {\n  const listed = await bridge fs.read_dir(\"data\")\n  const picked = await bridge fs.read_file(\"data/picked.txt\")\n  return match (listed) {\n    Ok(entries) => match (picked) {\n      Ok(bytes) => match (unsafe { String(entries.length) + \"/\" + String(bytes.length) }) {\n        Ok(text) => Ok([PostParam { slug: text }]),\n        Err(e) => Err(\"shape failed\")\n      },\n      Err(e) => Err(e)\n    },\n    Err(e) => Err(e)\n  }\n}\nexport const staticParams: Array<PostParam> = build {\n  return await slugs()\n}\nexport fn Page(props: PageProps) {\n  const marker = match (unsafe { staticParams[0].slug }) {\n    Ok(v) => v,\n    Err(e) => \"?\"\n  }\n  return <article><h1>{marker}</h1></article>;\n}\n"
}

fn get_status(port: u16, path: &str) -> Option<u16> {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?
        .get(format!("http://127.0.0.1:{port}{path}"))
        .send()
        .ok()
        .map(|res| res.status().as_u16())
}

fn get_body(port: u16, path: &str) -> Option<String> {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?
        .get(format!("http://127.0.0.1:{port}{path}"))
        .send()
        .ok()?
        .text()
        .ok()
}

/// Polls `probe` until it returns true or the deadline passes.
fn wait_until(deadline_secs: u64, mut probe: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(deadline_secs);
    while Instant::now() < deadline {
        if probe() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

#[test]
fn dev_watch_rematerializes_affected_slots_on_local_changes() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_security(
        project.path(),
        r#"{"allow": {"read": ["./data"]}, "prompt": false}"#,
    );
    write_slug_page(project.path(), slug_page_count_over_len());
    let data = project.path().join("data");
    fs::create_dir_all(&data).expect("mkdir data");
    fs::write(data.join("picked.txt"), "hello").expect("write picked");
    fs::write(data.join("other.txt"), "other").expect("write other");

    let port = free_port();
    let log_path = project.path().join("dev.log");
    let log = fs::File::create(&log_path).expect("dev.log");
    let child = Command::new(cli_bin())
        .args(["dev", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(project.path())
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka dev");
    let mut child = KillOnDrop(Some(child));
    let dev_log = || fs::read_to_string(&log_path).unwrap_or_default();
    // Any /posts/<slug> renders dynamically; assert on page content, not
    // route existence. Initial marker: 2 entries, 5 bytes -> "2/5".
    let marker_served = |marker: &str| {
        get_body(port, "/posts/anything").is_some_and(|body| body.contains(marker))
    };

    assert!(
        wait_until(90, || get_status(port, "/").is_some_and(|status| status == 200)),
        "deka dev did not become ready:\n{}",
        dev_log()
    );
    assert!(
        wait_until(30, || marker_served(">2/5<")),
        "initial materialization must render marker 2/5:\n{}",
        dev_log()
    );

    // Edit the read input: the slot reruns and the byte-length component moves.
    fs::write(data.join("picked.txt"), "hello!").expect("edit picked");
    assert!(
        wait_until(60, || marker_served(">2/6<")),
        "editing data/picked.txt must rematerialize the slot and render 2/6:\n{}",
        dev_log()
    );

    // Add a listed file: the directory-listing component moves.
    fs::write(data.join("gamma.txt"), "gamma").expect("write gamma");
    assert!(
        wait_until(60, || marker_served(">3/6<")),
        "adding data/gamma.txt must rematerialize the slot and render 3/6:\n{}",
        dev_log()
    );

    // Remove a listed file: the listing component moves back.
    fs::remove_file(data.join("gamma.txt")).expect("remove gamma");
    assert!(
        wait_until(60, || marker_served(">2/6<")),
        "removing data/gamma.txt must rematerialize the slot and render 2/6 again:\n{}",
        dev_log()
    );

    let log_text = dev_log();
    assert!(
        log_text.contains("invalidating build slots"),
        "targeted invalidation must be a deliberate, logged decision:\n{log_text}"
    );
    assert!(
        !log_text.contains("coarse build invalidation"),
        "exact observation matching must not fall back to a coarse rebuild:\n{log_text}"
    );

    // The dev manifest records the observations the invalidation used.
    let dev_manifest_path = project
        .path()
        .join("ds_modules")
        .join(".cache")
        .join("dev")
        .join("build-manifest.json");
    let dev_manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&dev_manifest_path).expect("read dev build manifest"),
    )
    .expect("parse dev build manifest");
    let observations = dev_manifest["slots"][0]["observations"]
        .as_array()
        .expect("observations");
    assert!(
        observations.iter().any(|observation| {
            observation["kind"] == "directory_listing" && observation["path"] == "data"
        }),
        "dev manifest must record the directory listing observation: {dev_manifest}"
    );
    assert!(
        observations.iter().any(|observation| {
            observation["kind"] == "read" && observation["path"] == "data/picked.txt"
        }),
        "dev manifest must record the read observation: {dev_manifest}"
    );

    if let Some(mut serve_child) = child.0.take() {
        let _ = serve_child.kill();
        let _ = serve_child.wait();
    }
}
