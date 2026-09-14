//! deka#1034 part 1: `StaticServeConfig::load` used to swallow a malformed
//! `serve.json` (headers/rewrites/redirects) via `unwrap_or_default()` — a
//! typo in a `headers` block meant the project's declared headers were
//! simply never applied, with no diagnostic, and the server started and
//! served normally anyway. These tests drive the real built `deka` binary,
//! not the resolver function directly.

use reqwest::blocking::Client;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

struct ServeProcess(Child);

impl Drop for ServeProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read ephemeral port")
        .port()
}

fn write_project(root: &Path, serve_json: &str) {
    fs::write(root.join("index.html"), "<html><body>hi</body></html>").unwrap();
    fs::write(root.join("serve.json"), serve_json).unwrap();
}

/// Wait for `child` to exit within `deadline`, without ever blocking forever
/// the way `Child::wait`/`Command::output` would if the fix regresses and
/// the server starts serving instead of failing startup. Kills and fails
/// loudly on timeout rather than hanging the suite.
fn wait_with_deadline(child: &mut Child, deadline: Duration) -> ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll child status") {
            return status;
        }
        if start.elapsed() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "process did not exit within {:?} — a malformed serve.json must fail \
                 startup, not hang serving with silently-defaulted config",
                deadline
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn read_all(child: &mut Child) -> String {
    use std::io::Read;
    let mut out = String::new();
    if let Some(stdout) = child.stdout.as_mut() {
        let _ = stdout.read_to_string(&mut out);
    }
    let mut err = String::new();
    if let Some(stderr) = child.stderr.as_mut() {
        let _ = stderr.read_to_string(&mut err);
    }
    out + &err
}

#[test]
fn malformed_serve_json_headers_block_fails_startup_with_a_named_diagnostic() {
    let root = tempfile::tempdir().expect("create project");
    write_project(
        root.path(),
        r#"{"headers":[{"source":"**/*.html","headers":"oops-not-an-array"}]}"#,
    );

    let port = free_port();
    let mut child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn deka serve");

    let status = wait_with_deadline(&mut child, Duration::from_secs(15));
    let text = read_all(&mut child);

    assert!(
        !status.success(),
        "a malformed serve.json headers block must fail startup, not serve with defaults: {text}"
    );
    assert!(
        text.contains("invalid serve config"),
        "diagnostic should name the problem: {text}"
    );
    assert!(
        text.contains("serve.json"),
        "diagnostic should name the file: {text}"
    );
}

#[test]
fn malformed_serve_json_syntax_fails_startup_non_zero() {
    let root = tempfile::tempdir().expect("create project");
    write_project(root.path(), r#"{"headers": [}"#);

    let port = free_port();
    let mut child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn deka serve");

    let status = wait_with_deadline(&mut child, Duration::from_secs(15));
    assert!(
        !status.success(),
        "syntactically invalid serve.json must exit non-zero"
    );
}

#[test]
fn valid_serve_json_with_headers_rewrites_redirects_still_serves() {
    // Fields not yet wired into the request path (deka#1034 follow-up
    // finding) are out of scope here; this proves the fix does not
    // regress the previously-working case of a *valid* config with these
    // blocks present — no false-positive failure, server starts and
    // serves normally.
    let root = tempfile::tempdir().expect("create project");
    write_project(
        root.path(),
        r#"{
            "headers": [
                {"source": "**/*.html", "headers": [{"key": "X-Test", "value": "1"}]}
            ],
            "rewrites": [
                {"source": "/foo", "destination": "/index.html"}
            ],
            "redirects": [
                {"source": "/old", "destination": "/index.html"}
            ]
        }"#,
    );

    let port = free_port();
    let mut command = Command::new(cli_bin());
    command
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let _server = ServeProcess(command.spawn().expect("spawn deka serve"));

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("build HTTP client");
    let deadline = Instant::now() + Duration::from_secs(20);
    #[allow(unused_assignments)]
    let mut last_err = String::from("no response yet");
    loop {
        match client
            .get(format!("http://127.0.0.1:{port}/index.html"))
            .send()
        {
            Ok(response) if response.status().is_success() => return,
            Ok(response) => last_err = format!("status {}", response.status()),
            Err(err) => last_err = err.to_string(),
        }
        if Instant::now() >= deadline {
            panic!("server with a valid serve.json never served successfully: {last_err}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
