//! deka#946: islands hydration pipeline.
//!
//! Real topology: the CLI binary serves a fixture whose layout marks
//! ThemeToggle and NewsletterSignup with `client:load`. The compiled DSX
//! components hydrate through `@js/react-dom/client` hydrateRoot — not a
//! parallel vanilla implementation.

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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/islands-hydration")
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

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("dst");
    for entry in fs::read_dir(src).expect("read src") {
        let entry = entry.expect("entry");
        let dest = dst.join(entry.file_name());
        if entry.file_type().expect("ty").is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).expect("copy file");
        }
    }
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
        .env_remove("https_proxy");
    if let Ok(dsc) = std::env::var("DEKA_DSC") {
        command.env("DEKA_DSC", dsc);
    } else if let Some(dsc) = compiler::dsc::find_dsc().ok().flatten() {
        command.env("DEKA_DSC", dsc);
    }
    command
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log));
    ServeProcess {
        child: command.spawn().expect("spawn deka serve"),
        log_path,
    }
}

fn wait_body(client: &Client, url: &str, log_path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
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
fn serve_emits_island_markers_and_shared_bundle() {
    let root = tempfile::tempdir().expect("temp project");
    copy_tree(&fixture_src(), root.path());
    assert!(!root.path().join("ds_modules").exists());

    let port = free_port();
    let server = spawn_serve(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("http client");
    let body = wait_body(
        &client,
        &format!("http://127.0.0.1:{port}/"),
        &server.log_path,
    );

    assert!(
        body.contains("data-deka-island=\"ThemeToggle\""),
        "SSR must wrap the theme island:\n{body}"
    );
    assert!(
        body.contains("data-deka-island=\"NewsletterSignup\""),
        "SSR must wrap the newsletter island:\n{body}"
    );
    assert!(
        body.contains("id=\"theme-toggle\""),
        "compiled ThemeToggle must SSR:\n{body}"
    );
    assert!(
        body.contains("id=\"newsletter\""),
        "compiled NewsletterSignup must SSR:\n{body}"
    );
    assert!(
        body.contains("src=\"/assets/islands.js\""),
        "document must inject the shared islands bundle:\n{body}"
    );
    assert!(
        !body.contains("islands.js") || !body.contains("vanilla"),
        "{body}"
    );
    for probe in pool::js_builtins::DEV_BYTE_PROBES {
        assert!(
            !body.contains(probe),
            "served HTML leaked development React bytes ({probe})"
        );
    }

    let bundle = wait_body(
        &client,
        &format!("http://127.0.0.1:{port}/assets/islands.js"),
        &server.log_path,
    );
    assert!(
        bundle.contains("function ThemeToggle"),
        "shared bundle must include the compiled DSX ThemeToggle:\n{}",
        &bundle[..bundle.len().min(500)]
    );
    assert!(
        bundle.contains("function NewsletterSignup"),
        "shared bundle must include the compiled DSX NewsletterSignup"
    );
    assert!(
        bundle.contains("hydrateRoot"),
        "shared bundle must call hydrateRoot"
    );
    assert!(
        bundle.contains("__deka_js_builtins[\"react-dom/client\"]")
            || bundle.contains("exports.hydrateRoot"),
        "shared bundle must inline production react-dom/client"
    );
    assert!(
        !bundle.contains("from \"@js/react") && !bundle.contains("from '@js/react"),
        "production islands bundle must inline @js/react* builtins"
    );
    for probe in pool::js_builtins::DEV_BYTE_PROBES {
        assert!(
            !bundle.contains(probe),
            "islands bundle leaked development React bytes ({probe})"
        );
    }
}

#[test]
fn client_builtin_resolves_without_install() {
    let js = r#"
import { hydrateRoot, createRoot } from "@js/react-dom/client";
export function boot(node) { return typeof hydrateRoot === "function" && typeof createRoot === "function"; }
"#;
    let inlined = pool::js_builtins::inline_into(js).expect("inline client");
    assert!(inlined.contains("hydrateRoot"));
    assert!(inlined.contains("createRoot"));
    for probe in pool::js_builtins::DEV_BYTE_PROBES {
        assert!(!inlined.contains(probe), "client inline leaked {probe}");
    }
}
