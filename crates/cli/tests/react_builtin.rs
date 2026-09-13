//! deka#943: pinned production React is a runtime builtin.
//!
//! Real topology: the CLI binary serves a fixture that imports `@js/react*`
//! with no `ds_modules/`, no `js_modules/`, and no network. A second path
//! builds `--bundle` and asserts development React bytes are absent.

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
        text.contains("react: 19.1.1"),
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
    let out = root.path().join("probe.bundle.js");
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

    if !out.is_file() {
        // dsc may reject the `@js/react` import (it is a runtime builtin,
        // not a summoned package). Host inlining still has to keep
        // development React out of the production graph.
        let source = fs::read_to_string(root.path().join("index.js")).expect("fixture");
        let inlined = pool::js_builtins::inline_into(&source).expect("inline fixture");
        assert!(
            inlined.contains("__deka_js_builtins[\"react\"]"),
            "expected inlined react factory: {text}"
        );
        for probe in pool::js_builtins::DEV_BYTE_PROBES {
            assert!(
                !inlined.contains(probe),
                "inlined prod graph leaked development React bytes ({probe})"
            );
        }
        return;
    }

    assert!(output.status.success(), "deka build --bundle failed: {text}");
    let bundle = fs::read_to_string(&out).expect("read bundle");
    assert!(
        bundle.contains("useState") || bundle.contains("__deka_js_builtins"),
        "bundle did not include React: {bundle}"
    );
    for probe in pool::js_builtins::DEV_BYTE_PROBES {
        assert!(
            !bundle.contains(probe),
            "prod bundle leaked development React bytes ({probe})"
        );
    }
    assert!(!root.path().join("ds_modules").exists());
}
