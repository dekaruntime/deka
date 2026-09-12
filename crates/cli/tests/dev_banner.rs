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

#[path = "support/app_router.rs"]
mod app_router;
fn init_project(dir: &Path) {
    app_router::init_project(dir);
    fs::write(dir.join("public/ready.txt"), "source-project-ready").unwrap();
}

fn serve_source_project(command: &str, root: &Path) {
    let port = free_port();
    let log_path = root.join(format!("{command}.log"));
    let log = fs::File::create(&log_path).expect("command log");
    let mut server = Command::new(cli_bin());
    server
        .args([command, ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root)
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone command log")))
        .stderr(Stdio::from(log));
    let dsc_beside_cli = Path::new(cli_bin()).with_file_name("dsc");
    if dsc_beside_cli.is_file() {
        server.env("DEKA_DSC", dsc_beside_cli);
    }
    let child = server.spawn().expect("spawn source-project server");
    let mut child = KillOnDrop(Some(child));

    let http = client();
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = String::new();
    while Instant::now() < deadline {
        if let Ok(response) = http
            .get(format!("http://127.0.0.1:{port}/ready.txt"))
            .send()
        {
            let status = response.status().as_u16();
            let body = response.text().unwrap_or_default();
            if status == 200 && body == "source-project-ready" {
                if let Some(mut server) = child.0.take() {
                    let _ = server.kill();
                    let _ = server.wait();
                }
                return;
            }
            last = format!("status={status}, body={body}");
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    let log = fs::read_to_string(&log_path).unwrap_or_default();
    panic!(
        "deka {command} did not serve the source app-router project on port {port}: {last}\nlog:\n{log}"
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
        text.lines()
            .any(|line| line.split_whitespace().next() == Some("dev")),
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
fn source_app_router_project_serves_with_dev_and_serve() {
    let root = TempDir::new().expect("temp project");
    init_project(root.path());

    // These commands are distinct postures in RFD 54, but before the later
    // artifact work lands they must both continue serving this source tree.
    for command in ["dev", "serve"] {
        serve_source_project(command, root.path());
    }
}

#[test]
fn deka_dev_prints_banner_serves_http_and_hmr() {
    let root = TempDir::new().expect("tempdir");
    init_project(root.path());
    let port = free_port();
    let log_path = root.path().join("dev.log");
    let log = fs::File::create(&log_path).expect("dev.log");
    let cwd = root
        .path()
        .canonicalize()
        .unwrap_or_else(|_| root.path().to_path_buf());

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
        if let Ok(res) = http
            .get(format!("http://127.0.0.1:{port}/ready.txt"))
            .send()
        {
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

    let ready_asset = http
        .get(format!("http://127.0.0.1:{port}/ready.txt"))
        .send()
        .expect("GET /ready.txt");
    assert_eq!(
        ready_asset.status().as_u16(),
        200,
        "readiness asset should be 200"
    );
    assert_eq!(
        ready_asset.text().expect("read readiness asset"),
        "source-project-ready"
    );

    let stylesheet = http
        .get(format!("http://127.0.0.1:{port}/style.css"))
        .send()
        .expect("GET /style.css");
    assert_eq!(
        stylesheet.status().as_u16(),
        200,
        "public/style.css from a freshly initialized project must be served"
    );
    assert!(
        stylesheet
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/css")),
        "public CSS must retain its content type"
    );

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
