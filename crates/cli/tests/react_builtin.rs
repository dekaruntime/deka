//! deka#943: pinned production React is a runtime builtin.
//!
//! Real topology: the CLI binary serves a fixture that imports `@js/react*`
//! with no installed packages and no network. A second path runs `deka
//! build --bundle` on a real `.ds` app and asserts the emitted bundle runs
//! with production React only.
//!
//! deka#1065: the compiler cache nests under `ds_modules/.cache`, so
//! `ds_modules` legitimately exists once the compiler runs even with zero
//! installed packages -- these tests assert the property that actually
//! matters (no installed-package content, i.e. no @js/react* materialized
//! as a real dependency), not bare directory existence.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// deka#1065: ds_modules holds the compiler cache (ds_modules/.cache), so
/// its bare existence no longer means "packages were installed". The
/// property this suite actually guards is that pinned production React
/// resolves as a runtime builtin with zero installed-package content --
/// mirrors deka_host::validation::modules::has_installed_modules.
fn ds_modules_has_no_installed_packages(root: &Path) -> bool {
    let ds_modules = root.join("ds_modules");
    if !ds_modules.is_dir() {
        return true;
    }
    fs::read_dir(&ds_modules)
        .expect("read ds_modules")
        .filter_map(|entry| entry.ok())
        .all(|entry| entry.file_name() == ".cache")
}

fn fixture_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/react-builtin")
}

/// The frozen development React source `deka dev`'s Fast Refresh serves
/// (crates/http/vendor/react/, gated behind the `dev-server` feature). The
/// default-feature CLI legitimately embeds this in its own binary (deka#980)
/// so the dev server has it to serve — that is not a leak. What must never
/// happen is this source landing inside a *production bundle* a merchant
/// actually ships. `dev_react_chunks()` slices it the same way
/// scripts/check-prod-react-bytes.sh does, so both guards use one
/// definition of "a chunk of this file."
fn dev_react_vendor_bytes() -> Vec<u8> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf();
    fs::read(workspace_root.join("crates/http/vendor/react/react-dom-client.js"))
        .expect("read crates/http/vendor/react/react-dom-client.js")
}

fn dev_react_chunks(vendor: &[u8]) -> Vec<&[u8]> {
    (0..vendor.len().saturating_sub(128))
        .step_by(4096)
        .map(|i| &vendor[i..i + 128])
        .collect()
}

struct ServeProcess {
    child: Child,
    log_path: PathBuf,
}

impl Drop for ServeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read ephemeral port")
        .port()
}

fn copy_fixture(dst: &Path) {
    fs::create_dir_all(dst).expect("fixture dir");
    for name in ["deka.json", "index.js", "probe.ds"] {
        fs::copy(fixture_src().join(name), dst.join(name)).expect("copy fixture file");
    }
    fs::write(
        dst.join("deka.lock"),
        r#"{"lockfileVersion":1,"packages":{}}"#,
    )
    .expect("lockfile");
}

fn spawn_serve(root: &Path, port: u16) -> ServeProcess {
    let log_path = root.join("serve.log");
    let log = fs::File::create(&log_path).expect("create serve log");
    let mut command = Command::new(cli_bin());
    command
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log));
    ServeProcess {
        child: command.spawn().expect("spawn deka serve"),
        log_path,
    }
}

fn wait_body(client: &Client, url: &str, log_path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = String::new();
    while Instant::now() < deadline {
        match client.get(url).send() {
            Ok(res) if res.status().as_u16() == 200 => {
                return res.text().expect("read body");
            }
            Ok(res) => last = format!("status {}", res.status()),
            Err(err) => last = err.to_string(),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let log = fs::read_to_string(log_path).unwrap_or_default();
    panic!("deka serve did not become ready: {last}\nserve.log:\n{log}");
}

#[test]
fn verbose_version_records_the_react_pin() {
    let output = Command::new(cli_bin())
        .args(["--version", "--verbose"])
        .env("NO_COLOR", "1")
        .output()
        .expect("deka --version --verbose");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains(&format!("react: {}", pool::js_builtins::REACT_VERSION)),
        "release metadata must surface the React pin: {text}"
    );
}

#[test]
fn serve_ssr_fixture_without_ds_modules_or_network() {
    let root = tempfile::tempdir().expect("temp project");
    copy_fixture(root.path());
    assert!(!root.path().join("ds_modules").exists());
    assert!(!root.path().join("js_modules").exists());

    let port = free_port();
    let server = spawn_serve(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http client");
    let body = wait_body(
        &client,
        &format!("http://127.0.0.1:{port}/"),
        &server.log_path,
    );

    assert!(body.contains("id=\"counter\""), "{body}");
    assert!(body.contains("data-island=\"counter\""), "{body}");
    assert!(body.contains(">7<"), "{body}");
    assert!(body.contains("id=\"effect\""), "{body}");
    assert!(body.contains(">ssr<"), "{body}");
    assert!(body.contains("data-theme=\"dark\""), "{body}");
    assert!(!body.contains("Minified React error"), "{body}");
    for probe in pool::js_builtins::DEV_BYTE_PROBES {
        assert!(
            !body.contains(probe),
            "served SSR leaked development React bytes ({probe}): {body}"
        );
    }
    // deka#1065: ds_modules now legitimately exists to hold the compiler
    // cache -- the property this test actually guards is that serving the
    // fixture never installed @js/react* (or anything else) as a real
    // package.
    assert!(
        ds_modules_has_no_installed_packages(root.path()),
        "serving a pure-builtin fixture must not add installed-package content to ds_modules"
    );
    assert!(!root.path().join("js_modules").exists());
}

#[test]
fn build_bundle_inlines_prod_react_without_dev_bytes() {
    let root = tempfile::tempdir().expect("temp project");
    copy_fixture(root.path());
    let out = root.path().join("probe.bundle.mjs");
    let output = Command::new(cli_bin())
        .args([
            "build",
            "probe.ds",
            "--bundle",
            "--out",
            out.to_str().expect("utf-8 out"),
        ])
        .current_dir(root.path())
        .env("NO_COLOR", "1")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .output()
        .expect("deka build --bundle");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        output.status.success(),
        "deka build --bundle failed: {text}"
    );
    assert!(
        out.is_file(),
        "deka build --bundle did not write {}: {text}",
        out.display()
    );
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        bundle.contains("__deka_js_builtins[\"react\"]"),
        "bundle did not inline production React: {bundle}"
    );
    assert!(
        !bundle.contains("from \"@js/react") && !bundle.contains("from '@js/react"),
        "bundle still imports @js/react* instead of inlining: {bundle}"
    );
    for probe in pool::js_builtins::DEV_BYTE_PROBES {
        assert!(
            !bundle.contains(probe),
            "prod bundle leaked development React bytes ({probe})"
        );
    }

    // deka#982: the checks above assert on `bundle`, an in-memory decoded
    // `String` — they cannot catch source that lands in the *file on disk*
    // through some path that never touches that `String` (encoding quirks,
    // a post-processing pass, a different write path). Re-read the emitted
    // file's raw bytes and scan them directly for disjoint chunks of the
    // actual frozen dev-React vendor source, the same technique
    // scripts/check-prod-react-bytes.sh uses on the CLI binary itself. This
    // is the byte-level guard on the artifact that matters: what a merchant
    // actually ships to their storefront's customers.
    let bundle_bytes = fs::read(&out).expect("read bundle bytes");
    let vendor = dev_react_vendor_bytes();
    let chunks = dev_react_chunks(&vendor);
    assert!(
        !chunks.is_empty(),
        "no vendor chunks derived from react-dom-client.js"
    );
    assert!(
        !chunks
            .iter()
            .any(|chunk| bundle_bytes.windows(chunk.len()).any(|w| w == *chunk)),
        "prod bundle file bytes contain a chunk of the frozen dev-React vendor source"
    );

    // deka#1065: ds_modules now legitimately exists to hold the compiler
    // cache -- assert no installed-package content, not bare absence.
    assert!(
        ds_modules_has_no_installed_packages(root.path()),
        "building the bundle must not add installed-package content to ds_modules"
    );

    let runner = root.path().join("run-bundle.mjs");
    fs::write(
        &runner,
        "import { pathToFileURL } from 'node:url';\n\
         const mod = await import(pathToFileURL(process.argv[2]));\n\
         if (typeof mod.probe !== 'function') {\n\
           throw new Error('probe export missing');\n\
         }\n",
    )
    .expect("write bundle runner");
    let run = Command::new("node")
        .args([
            runner.to_str().expect("utf-8 runner"),
            out.to_str().expect("utf-8 bundle"),
        ])
        .current_dir(root.path())
        .env("NO_COLOR", "1")
        .output()
        .expect("node must be available to execute the production bundle");
    let run_text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.status.success(),
        "emitted bundle failed to run: {run_text}"
    );
}

#[test]
fn explicit_jsx_runtime_without_jsx_literals_builds_and_executes() {
    assert_jsx_runtime_bundle(
        "manual-jsx.ds",
        r#"
const many = bundle.manualMany();
assert.equal(many.type, Symbol.for("react.fragment"));
assert.deepEqual(many.props.children, ["one", "two"]);
assert.equal(many.key, "7");
"#,
    );
}

#[test]
fn explicit_jsx_runtime_with_jsx_literals_builds_and_executes() {
    assert_jsx_runtime_bundle(
        "mixed-jsx.dsx",
        r#"
assert.equal(element.key, "manual-key");
const aliased = bundle.aliasedJsx();
assert.equal(aliased.type, "aside");
assert.equal(aliased.props.children, "aliased jsx");
const literal = literalJsx();
assert.equal(literal.type, "section");
assert.equal(literal.props.children, "literal jsx");
"#,
    );
}

fn assert_jsx_runtime_bundle(fixture: &str, extra_assertions: &str) {
    let root = tempfile::tempdir().expect("temp project");
    copy_fixture(root.path());
    fs::copy(fixture_src().join(fixture), root.path().join(fixture))
        .expect("copy manual JSX fixture");
    let out = root.path().join("manual-jsx.bundle.mjs");
    let output = Command::new(cli_bin())
        .args(["build", fixture, "--bundle", "--out"])
        .arg(&out)
        .current_dir(root.path())
        .env("DEKA_DSC", pinned_dsc())
        .env("NO_COLOR", "1")
        .output()
        .expect("deka build manual JSX");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let bundle = fs::read_to_string(&out).expect("emitted bundle");
    assert!(bundle.contains("__deka_js_builtins[\"react/jsx-runtime\"]"));
    assert!(!bundle.contains("__deka_js_builtin_react_jsx_runtime"));
    assert!(!root
        .path()
        .join("__deka_js_builtin_react_jsx_runtime.ds")
        .exists());
    let runner = root.path().join("run-manual.mjs");
    fs::write(
        &runner,
        format!(
            r#"
import {{ strict as assert }} from "node:assert";
import * as bundle from "./manual-jsx.bundle.mjs";
const {{ manualJsx, literalJsx }} = bundle;
const element = manualJsx();
assert.equal(element.type, "p");
assert.equal(element.props.children, "manual jsx");
{extra_assertions}
console.log("manual jsx: p / manual jsx");
"#
        ),
    )
    .expect("write runner");
    let run = Command::new("node")
        .arg(&runner)
        .output()
        .expect("execute bundle");
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "manual jsx: p / manual jsx"
    );
}

#[test]
fn probe_ds_compiles_with_pinned_dsc() {
    let root = tempfile::tempdir().expect("temp project");
    copy_fixture(root.path());
    let source = fs::read_to_string(root.path().join("probe.ds")).expect("probe.ds");
    let rewrite = pool::js_builtins::rewrite_for_dsc_transpile(&source);
    assert!(
        !rewrite.source.contains("@js/react"),
        "pinned dsc rejects explicit @js/react; rewrite must strip it: {}",
        rewrite.source
    );
    let stripped = root.path().join("probe.stripped.ds");
    fs::write(&stripped, &rewrite.source).expect("write stripped probe");
    let emitted = root.path().join("probe.stripped.js");
    let dsc = pinned_dsc();
    let output = Command::new(&dsc)
        .args([
            "transpile",
            stripped.to_str().expect("utf-8 stripped"),
            "--out",
            emitted.to_str().expect("utf-8 emitted"),
        ])
        .current_dir(root.path())
        .env("NO_COLOR", "1")
        .output()
        .expect("dsc transpile probe");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "probe.ds must compile through pinned dsc after treating @js/react as an external: {text}"
    );
    let js = fs::read_to_string(&emitted).expect("emitted js");
    assert!(
        js.contains("from \"@js/react\""),
        "dsc must re-inject the runtime builtin: {js}"
    );
    assert!(
        js.contains("export function probe()"),
        "dsc must emit probe: {js}"
    );
}

fn pinned_dsc() -> PathBuf {
    if let Ok(path) = std::env::var("DEKA_DSC") {
        return PathBuf::from(path);
    }
    compiler::dsc::find_dsc()
        .expect("resolve sibling/repository dsc")
        .expect("tests require dsc: set DEKA_DSC to the pinned compiler")
}
