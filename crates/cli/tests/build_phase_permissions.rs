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

/// RFD 27 (deka#755): app source may not name `bridge` — only grant-table
/// verified packages may. These fixtures exercise the *permission* layer
/// (allow/deny read targets), so the bridge calls live in this granted
/// dependency package and the app imports thin wrappers. The manifest field
/// is deliberately absent: a dependency's `host.kinds` is untrusted and
/// ignored; the grant table entry (DEKA_HOST_GRANTS) is the only authority.
const FIXTUREFS_SOURCE: &str = "export async fn read_file(p: string) Promise<Result<bytes, string>> {\n  return await bridge fs.read_file(p)\n}\n\nexport async fn read_dir(p: string) Promise<Result<Array<string>, string>> {\n  return await bridge fs.read_dir(p)\n}\n";

/// Writes the fixture package into the project's ds_modules/, pins its
/// fsGraph digest in deka.lock (the same shape `deka install` writes), and
/// returns the DEKA_HOST_GRANTS grant-table JSON keyed by that digest.
fn add_fixturefs(project: &Path) -> String {
    let package = project.join("ds_modules").join("@deka").join("fixturefs");
    fs::create_dir_all(&package).expect("mkdir fixturefs");
    fs::write(
        package.join("deka.json"),
        r#"{"name":"@deka/fixturefs","version":"1.0.0"}"#,
    )
    .expect("write fixturefs manifest");
    fs::write(package.join("index.ds"), FIXTUREFS_SOURCE).expect("write fixturefs source");

    let integrity =
        deka_host::integrity::compute_package_integrity(&package).expect("fixturefs integrity");
    let lock_path = project.join("deka.lock");
    let mut lock: serde_json::Value = fs::read_to_string(&lock_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({ "lockfileVersion": 1, "packages": {} }));
    lock["packages"]["@deka/fixturefs"] = serde_json::json!([
        "@deka/fixturefs@1.0.0",
        "linkhash:@deka/fixturefs",
        {
            "moduleGraph": { "algo": "sha256", "hash": integrity.module_graph },
            "fsGraph": { "algo": "sha256", "hash": integrity.fs_graph }
        },
        ""
    ]);
    fs::write(&lock_path, serde_json::to_string_pretty(&lock).expect("lock json"))
        .expect("write deka.lock");

    serde_json::to_string(&serde_json::json!([
        {
            "name": "@deka/fixturefs",
            "version": "1.0.0",
            "digest": integrity.fs_graph,
            "kinds": ["fs"]
        }
    ]))
    .expect("grant table json")
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

fn run_build(dir: &Path, host_grants: &str) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(dir)
        .env("DEKA_HOST_GRANTS", host_grants)
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn run_with_args(dir: &Path, args: &[&str], host_grants: &str) -> (bool, String) {
    let output = Command::new(cli_bin())
        .args(args)
        .current_dir(dir)
        .env("DEKA_HOST_GRANTS", host_grants)
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

/// Writes deka.json with a security section; keeps the scaffold's shape. The
/// `@deka/fixturefs` dependency is declared here because dsc validates
/// imports against deka.json + deka.lock before compiling.
fn write_security(project: &Path, security: &str) {
    fs::write(
        project.join("deka.json"),
        format!(
            "{{\n  \"name\": \"phase\",\n  \"type\": \"serve\",\n  \"serve\": {{ \"mode\": \"ds\" }},\n  \"dependencies\": {{ \"@deka/fixturefs\": \"1.0.0\" }},\n  \"security\": {security}\n}}\n"
        ),
    )
    .expect("write deka.json");
}

/// Writes deka.json with a phase-aware `permissions` block (deka#757, RFD 53)
/// instead of the legacy `security` block: `permissions.dev.build` is the only
/// authority a build slot executes under, independent from request-time dev.
fn write_permissions(project: &Path, permissions: &str) {
    fs::write(
        project.join("deka.json"),
        format!(
            "{{\n  \"name\": \"phase\",\n  \"type\": \"serve\",\n  \"serve\": {{ \"mode\": \"ds\" }},\n  \"dependencies\": {{ \"@deka/fixturefs\": \"1.0.0\" }},\n  \"permissions\": {permissions}\n}}\n"
        ),
    )
    .expect("write deka.json");
}

/// A [slug] page whose build body reads `path` and propagates failures. The
/// bridge call lives in the granted `@deka/fixturefs` package (RFD 27: app
/// source cannot name `bridge`); the build block's first statement stays on
/// line 10, which is the source location the permission diagnostic links.
fn slug_page_probing(path: &str) -> String {
    format!(
        "import {{ read_file }} from \"@deka/fixturefs\"\ninterface PageProps {{ slug: string }}\nstruct PostParam {{ slug: string }}\nasync fn probe(p: string) Promise<Result<Array<PostParam>, string>> {{\n  const res = await read_file(p)\n  return match (res) {{\n    Ok(v) => Ok([PostParam {{ slug: \"hello\" }}]),\n    Err(e) => Err(e) }}\n}}\nexport const staticParams: Array<PostParam> = build {{\n  return await probe(\"{path}\")\n}}\nexport fn Page(props: PageProps) {{\n  return <article><h1>{{props.slug}}</h1></article>;\n}}\n"
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
    let grants = add_fixturefs(project.path());
    write_slug_page(project.path(), &slug_page_probing("data/x.json"));

    let (success, combined) = run_build(project.path(), &grants);
    assert!(
        !success,
        "deka build must fail when the build body reads a file the policy denies: {combined}"
    );
    assert!(
        combined.contains(runtime_core::host_bridge::PERMISSION_DENIED_MARKER),
        "failure must carry the machine-readable permission diagnostic (RFD 27): {combined}"
    );
    assert!(
        combined.contains(r#""capability":"read""#),
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
    let grants = add_fixturefs(project.path());
    let slug_dir = project.path().join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    // One successful read, one directory listing, one absent path — all via
    // the granted @deka/fixturefs wrappers (RFD 27: app source cannot name
    // `bridge` directly).
    fs::write(
        slug_dir.join("page.dsx"),
        "import { read_file, read_dir } from \"@deka/fixturefs\"\ninterface PageProps { slug: string }\nstruct PostParam { slug: string }\nexport const staticParams: Array<PostParam> = build {\n  const present = await read_file(\"data/slugs/a.txt\")\n  const listed = await read_dir(\"data\")\n  const missing = await read_file(\"data/missing.txt\")\n  return Ok([PostParam { slug: \"hello\" }])\n}\nexport fn Page(props: PageProps) {\n  return <article><h1>{props.slug}</h1></article>;\n}\n",
    )
    .expect("write slug page");
    fs::create_dir_all(project.path().join("data").join("slugs")).expect("mkdir data/slugs");
    fs::write(project.path().join("data").join("slugs").join("a.txt"), "a").expect("write data");

    let (success, combined) = run_build(project.path(), &grants);
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
    let grants = add_fixturefs(project.path());
    write_slug_page(project.path(), &slug_page_probing("data/x.json"));

    let (success, combined) = run_build(project.path(), &grants);
    assert!(
        !success,
        "deka build must fail when the policy denies the build body's read: {combined}"
    );
    assert!(
        combined.contains(runtime_core::host_bridge::PERMISSION_DENIED_MARKER),
        "failure must carry the machine-readable permission diagnostic (RFD 27): {combined}"
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
    let grants = add_fixturefs(project.path());
    fs::write(
        project.path().join("main.ds"),
        "import { read_file } from \"@deka/fixturefs\"\nasync fn go() Promise<string> {\n  const res = await read_file(\"data/x.json\")\n  return match (res) {\n    Ok(v) => \"runtime-read-ok\",\n    Err(e) => \"runtime-read-denied\"\n  }\n}\nasync fn main() Promise<string> {\n  const r = await go()\n  unsafe { console.log(r) }\n  return r\n}\nmain()\n",
    )
    .expect("write main.ds");

    let (_, combined) = run_with_args(project.path(), &["run", "main.ds", "--no-prompt"], &grants);
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
    let grants = add_fixturefs(project.path());
    fs::create_dir_all(project.path().join("data")).expect("mkdir data");
    fs::write(project.path().join("data").join("x.json"), "{}").expect("write data");
    fs::write(
        project.path().join("main.ds"),
        "import { read_file } from \"@deka/fixturefs\"\nasync fn go() Promise<string> {\n  const res = await read_file(\"data/x.json\")\n  return match (res) {\n    Ok(v) => \"runtime-read-ok\",\n    Err(e) => \"runtime-read-denied\"\n  }\n}\nasync fn main() Promise<string> {\n  const r = await go()\n  unsafe { console.log(r) }\n  return r\n}\nmain()\n",
    )
    .expect("write main.ds");

    let (_, combined) = run_with_args(project.path(), &["run", "main.ds", "--no-prompt"], &grants);
    assert!(
        combined.contains("runtime-read-ok"),
        "an explicit read allow must unlock runtime reads just as it unlocks build reads: {combined}"
    );

    write_slug_page(project.path(), &slug_page_probing("data/x.json"));
    let (success, combined) = run_build(project.path(), &grants);
    assert!(
        success,
        "deka build should succeed under the same explicit allow: {combined}"
    );
}

#[test]
fn build_body_cannot_inherit_request_time_dev_grants() {
    // The property that matters (rfd#48): a build body must not gain
    // capabilities merely because the host evaluates it. This manifest grants
    // the read to request-time dev — and nothing to the build phase — so the
    // build body's fs read must be denied with the typed diagnostic even
    // though the same project, at request time, may read the file.
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_permissions(project.path(), r#"{"dev": {"read": ["./data"]}}"#);
    let grants = add_fixturefs(project.path());
    write_slug_page(project.path(), &slug_page_probing("data/x.json"));

    let (success, combined) = run_build(project.path(), &grants);
    assert!(
        !success,
        "deka build must fail when only request-time dev holds the read grant: {combined}"
    );
    assert!(
        combined.contains(runtime_core::host_bridge::PERMISSION_DENIED_MARKER),
        "failure must carry the machine-readable permission diagnostic (RFD 27): {combined}"
    );
    assert!(
        combined.contains(r#""capability":"read""#),
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

    // An explicit `build: false` is the same policy as omitting the key: the
    // request-time grant still must not leak into the build phase.
    write_permissions(
        project.path(),
        r#"{"dev": {"read": ["./data"], "build": false}}"#,
    );
    let (success, combined) = run_build(project.path(), &grants);
    assert!(
        !success,
        "explicit build:false must deny the build body just like an omitted key: {combined}"
    );
    assert!(
        combined.contains(runtime_core::host_bridge::PERMISSION_DENIED_MARKER),
        "explicit build:false denial must stay machine-readable: {combined}"
    );
}

#[test]
fn phase_aware_dev_build_grant_unlocks_build_and_records_inputs() {
    // Independence in the other direction: the only read grant in this
    // manifest lives in `permissions.dev.build`. The build body reads
    // successfully (a present read, a directory listing, an absent path) and
    // the manifest records all three observations, while ordinary runtime
    // code under the same manifest stays denied — the build phase widens
    // nothing for anyone else (rfd#48 parity).
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_permissions(
        project.path(),
        r#"{"dev": {"build": {"read": ["./data"]}}}"#,
    );
    let grants = add_fixturefs(project.path());
    let slug_dir = project.path().join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    fs::write(
        slug_dir.join("page.dsx"),
        "import { read_file, read_dir } from \"@deka/fixturefs\"\ninterface PageProps { slug: string }\nstruct PostParam { slug: string }\nexport const staticParams: Array<PostParam> = build {\n  const present = await read_file(\"data/slugs/a.txt\")\n  const listed = await read_dir(\"data\")\n  const missing = await read_file(\"data/missing.txt\")\n  return Ok([PostParam { slug: \"hello\" }])\n}\nexport fn Page(props: PageProps) {\n  return <article><h1>{props.slug}</h1></article>;\n}\n",
    )
    .expect("write slug page");
    fs::create_dir_all(project.path().join("data").join("slugs")).expect("mkdir data/slugs");
    fs::write(project.path().join("data").join("slugs").join("a.txt"), "a").expect("write data");

    let (success, combined) = run_build(project.path(), &grants);
    assert!(
        success,
        "deka build should succeed when permissions.dev.build grants the reads: {combined}"
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

    // Parity: `deka run` selects the prod profile, which this manifest leaves
    // fully denied — the same read is not available to ordinary runtime code.
    fs::write(
        project.path().join("main.ds"),
        "import { read_file } from \"@deka/fixturefs\"\nasync fn go() Promise<string> {\n  const res = await read_file(\"data/slugs/a.txt\")\n  return match (res) {\n    Ok(v) => \"runtime-read-ok\",\n    Err(e) => \"runtime-read-denied\"\n  }\n}\nasync fn main() Promise<string> {\n  const r = await go()\n  unsafe { console.log(r) }\n  return r\n}\nmain()\n",
    )
    .expect("write main.ds");
    let (_, combined) = run_with_args(project.path(), &["run", "main.ds", "--no-prompt"], &grants);
    assert!(
        combined.contains("runtime-read-denied"),
        "runtime fs read must be denied under the default-deny prod profile: {combined}"
    );
    assert!(
        !combined.contains("runtime-read-ok"),
        "runtime fs read must not succeed under the default-deny prod profile: {combined}"
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
/// omits helpers referenced only inside `unsafe` arrows. The bridge calls
/// live in the granted `@deka/fixturefs` package (RFD 27).
fn slug_page_count_over_len() -> &'static str {
    "import { read_file, read_dir } from \"@deka/fixturefs\"\ninterface PageProps { slug: string }\nstruct PostParam { slug: string }\nasync fn slugs() Promise<Result<Array<PostParam>, string>> {\n  const listed = await read_dir(\"data\")\n  const picked = await read_file(\"data/picked.txt\")\n  return match (listed) {\n    Ok(entries) => match (picked) {\n      Ok(bytes) => match (unsafe { String(entries.length) + \"/\" + String(bytes.length) }) {\n        Ok(text) => Ok([PostParam { slug: text }]),\n        Err(e) => Err(\"shape failed\")\n      },\n      Err(e) => Err(e)\n    },\n    Err(e) => Err(e)\n  }\n}\nexport const staticParams: Array<PostParam> = build {\n  return await slugs()\n}\nexport fn Page(props: PageProps) {\n  const marker = match (unsafe { staticParams[0].slug }) {\n    Ok(v) => v,\n    Err(e) => \"?\"\n  }\n  return <article><h1>{marker}</h1></article>;\n}\n"
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
    let grants = add_fixturefs(project.path());
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
        .env("DEKA_HOST_GRANTS", &grants)
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

/// Codex review of deka#729: slot ids embed the compiler span of the
/// `build {}` block, so inserting a line ABOVE the block changes the id.
/// A targeted refresh that filters the fresh plan by the manifest's old ids
/// would rematerialize nothing and serve stale (or unresolved) values. The
/// fix replans the changed source file and replaces its manifest slots.
#[test]
fn dev_watch_replans_a_source_file_when_the_build_block_span_shifts() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_security(
        project.path(),
        r#"{"allow": {"read": ["./data"]}, "prompt": false}"#,
    );
    let grants = add_fixturefs(project.path());
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
        .env("DEKA_HOST_GRANTS", &grants)
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka dev");
    let mut child = KillOnDrop(Some(child));
    let dev_log = || fs::read_to_string(&log_path).unwrap_or_default();
    let dev_manifest_path = project
        .path()
        .join("ds_modules")
        .join(".cache")
        .join("dev")
        .join("build-manifest.json");
    let slot_id = || -> Option<String> {
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&dev_manifest_path).ok()?).ok()?;
        manifest["slots"][0]["id"].as_str().map(str::to_string)
    };
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
    let id_before = slot_id().expect("manifest slot id before the edit");

    // Insert a blank line ABOVE the build block: the block's span shifts, so
    // its compiler slot id changes even though the build body is untouched.
    let page_path = project
        .path()
        .join("app")
        .join("posts")
        .join("[slug]")
        .join("page.dsx");
    let shifted = format!("\n{}", fs::read_to_string(&page_path).expect("read page"));
    fs::write(&page_path, shifted).expect("write shifted page");

    // The page must keep serving through the span shift: the file is
    // replanned, its (new-id) slot rematerialized, and its manifest slots
    // replaced wholesale. The pre-fix behavior left the new id unmaterialized
    // and the page unresolved.
    assert!(
        wait_until(60, || marker_served(">2/5<")),
        "a line inserted above the build block must replan the file and keep serving 2/5:\n{}",
        dev_log()
    );
    let id_after = slot_id().expect("manifest slot id after the edit");
    assert_ne!(
        id_before, id_after,
        "the fixture edit must actually shift the compiler slot id (regression condition)"
    );

    // The replaced slot must record fresh observations: editing the picked
    // file afterwards invalidates it through the NEW id.
    fs::write(data.join("picked.txt"), "hello!").expect("edit picked");
    assert!(
        wait_until(60, || marker_served(">2/6<")),
        "the replanned slot must observe data/picked.txt and rematerialize to 2/6:\n{}",
        dev_log()
    );

    let log_text = dev_log();
    assert!(
        log_text.contains("replanning build slots in [app/posts/[slug]/page.dsx]"),
        "a source-file change must be logged as a wholesale replan:\n{log_text}"
    );
    assert!(
        log_text.contains("invalidating build slots"),
        "targeted observation invalidation must stay a deliberate, logged decision:\n{log_text}"
    );
    assert!(
        !log_text.contains("coarse build invalidation"),
        "exact matching must not fall back to a coarse rebuild:\n{log_text}"
    );
    assert!(
        !log_text.contains("failed to rematerialize"),
        "no rematerialization in this scenario may fail:\n{log_text}"
    );

    if let Some(mut serve_child) = child.0.take() {
        let _ = serve_child.kill();
        let _ = serve_child.wait();
    }
}

/// Targeted `deka dev` invalidation under the phase-aware model (deka#757):
/// the only read grant in this manifest is `permissions.dev.build`, yet the
/// dev server still materializes the slot, and editing a recorded input
/// rematerializes exactly the affected slot — an actual content edit, not a
/// directory-glob re-run.
#[test]
fn dev_watch_invalidates_slots_under_phase_aware_permissions() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    write_permissions(
        project.path(),
        r#"{"dev": {"build": {"read": ["./data"]}}}"#,
    );
    let grants = add_fixturefs(project.path());
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
        .env("DEKA_HOST_GRANTS", &grants)
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka dev");
    let mut child = KillOnDrop(Some(child));
    let dev_log = || fs::read_to_string(&log_path).unwrap_or_default();
    // Initial marker: 2 entries, 5 bytes -> "2/5".
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
        "initial materialization must render marker 2/5 under permissions.dev.build:\n{}",
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

    let log_text = dev_log();
    assert!(
        log_text.contains("invalidating build slots"),
        "targeted invalidation must be a deliberate, logged decision:\n{log_text}"
    );
    assert!(
        !log_text.contains("coarse build invalidation"),
        "exact observation matching must not fall back to a coarse rebuild:\n{log_text}"
    );

    if let Some(mut serve_child) = child.0.take() {
        let _ = serve_child.kill();
        let _ = serve_child.wait();
    }
}
