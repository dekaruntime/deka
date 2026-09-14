//! deka#943: pinned production React is a runtime builtin.
//!
//! Real topology: the CLI binary serves a fixture that imports `@js/react*`
//! with no `ds_modules/`, no `js_modules/`, and no network. A second path
//! runs `deka build --bundle` on a real `.ds` app and asserts the emitted
//! bundle runs with production React only.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn fixture_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/react-builtin")
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
    assert!(!root.path().join("ds_modules").exists());
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
    assert!(!root.path().join("ds_modules").exists());

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
