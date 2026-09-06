//! `deka` always enables HTTP + HMR, prints the stdio ascii banner with
//! URL/cwd once at listen time, and caches under `ds_modules/.cache/dev`.

use reqwest::blocking::Client;
use reqwest::redirect;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

struct KillOnDrop(Option<Child>);

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

fn client() -> Client {
    Client::builder()
        .redirect(redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client")
}

fn init_project(dir: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(dir)
        .output()
        .expect("deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn help_lists_dev_command() {
    let output = Command::new(cli_bin())
        .arg("--help")
        .output()
        .expect("deka --help");
    assert!(output.status.success(), "deka --help should succeed");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.lines().any(|line| line.split_whitespace().next() == Some("dev")),
        "deka --help should list `dev`; output:\n{text}"
    );
}

#[test]
fn registry_exposes_dev_command() {
    let registry = cli::build_registry();
    let command = registry
        .command_named("dev")
        .expect("dev command should be registered");
    assert_eq!(command.name, "dev");
    assert_eq!(command.category, "runtime");
}

#[test]
fn deka_dev_prints_banner_serves_http_and_hmr() {
    let root = TempDir::new().expect("tempdir");
    init_project(root.path());
    let port = free_port();
    let log_path = root.path().join("dev.log");
    let log = fs::File::create(&log_path).expect("dev.log");
    let cwd = root.path().canonicalize().unwrap_or_else(|_| root.path().to_path_buf());

    let mut cmd = Command::new(cli_bin());
    cmd.args(["dev", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root.path())
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log));
    // Prefer an explicit dsc when present next to the test cli binary.
    let dsc_beside_cli = Path::new(cli_bin()).with_file_name("dsc");
    if dsc_beside_cli.is_file() {
        cmd.env("DEKA_DSC", &dsc_beside_cli);
    }
    let child = cmd.spawn().expect("spawn deka");
    let mut child = KillOnDrop(Some(child));

    let http = client();
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut ready = false;
    let mut last_status = None;
    let mut last_body = String::new();
    while Instant::now() < deadline {
        if let Ok(res) = http.get(format!("http://127.0.0.1:{port}/")).send() {
            let status = res.status().as_u16();
            last_status = Some(status);
            if status == 200 {
                ready = true;
                break;
            }
            last_body = res.text().unwrap_or_default();
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let log_text = fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        ready,
        "deka did not become ready on port {port} (last_status={last_status:?})\nbody:\n{last_body}\ndev.log:\n{log_text}"
    );

    let listen_url = format!("http://localhost:{port}");
    assert!(
        log_text.contains("[listen]") && log_text.contains(&listen_url),
        "expected [listen] {listen_url} in log:\n{log_text}"
    );
    // Terrace figlet banner is block glyphs (may not contain the letters
    // "deka"); accept either the art blocks or a literal "deka".
    assert!(
        log_text.contains('░') || log_text.to_lowercase().contains("deka"),
        "expected ascii/brand banner (figlet blocks or 'deka'):\n{log_text}"
    );
    assert!(
        log_text.contains(&listen_url),
        "expected listen URL in banner/log:\n{log_text}"
    );
    let cwd_display = cwd.display().to_string();
    assert!(
        log_text.contains(&cwd_display)
            || log_text.contains(root.path().to_string_lossy().as_ref()),
        "expected cwd in banner/log (cwd={cwd_display}):\n{log_text}"
    );

    let home = http
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .expect("GET /");
    assert_eq!(home.status().as_u16(), 200, "GET / should be 200");

    let hmr = http
        .get(format!("http://127.0.0.1:{port}/_deka/hmr"))
        .send()
        .expect("GET /_deka/hmr");
    assert_eq!(
        hmr.status().as_u16(),
        426,
        "HMR without upgrade must be 426"
    );

    let cache = root.path().join("ds_modules").join(".cache").join("dev");
    assert!(
        cache.is_dir(),
        "expected dev compiler cache at {}\nlog:\n{log_text}",
        cache.display()
    );

    if let Some(mut serve_child) = child.0.take() {
        let _ = serve_child.kill();
        let _ = serve_child.wait();
    }
}
