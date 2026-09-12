//! The default project must be safe to inspect and must follow the four
//! project-root conventions without extra setup.

use std::fs;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

#[test]
fn init_help_does_not_create_a_project() {
    let project = tempfile::tempdir().expect("tempdir");
    let output = Command::new(cli_bin())
        .args(["init", "--help"])
        .current_dir(project.path())
        .output()
        .expect("run deka init --help");

    assert!(
        output.status.success(),
        "deka init --help failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !project.path().join("deka.json").exists(),
        "deka init --help must not initialize a project"
    );
}

#[test]
fn init_creates_static_project_roots_and_preserves_existing_files() {
    let project = tempfile::tempdir().expect("tempdir");
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(project.path())
        .output()
        .expect("run deka init");
    assert!(output.status.success(), "deka init must succeed");

    for directory in ["app", "api", "src", "public"] {
        assert!(
            project.path().join(directory).is_dir(),
            "init must create {directory}/"
        );
    }

    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(project.path().join("deka.json")).unwrap())
            .unwrap();
    assert_eq!(config["serve"]["mode"], "static");
    assert_eq!(config["serve"]["entry"], "public");
    let page = project.path().join("public/index.html");
    assert!(
        fs::read_to_string(&page)
            .unwrap()
            .contains("<h1>Deka App</h1>")
    );
    assert!(project.path().join("public/style.css").is_file());
    assert!(!project.path().join("app/page.dsx").exists());

    fs::write(&page, "user-authored page").unwrap();
    assert!(
        Command::new(cli_bin())
            .arg("init")
            .current_dir(project.path())
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read_to_string(page).unwrap(), "user-authored page");
}

#[test]
fn fresh_init_serves_html_and_css_without_exposing_project_files() {
    use std::net::TcpListener;
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    struct Server(Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let project = tempfile::tempdir().unwrap();
    assert!(
        Command::new(cli_bin())
            .arg("init")
            .current_dir(project.path())
            .status()
            .unwrap()
            .success()
    );
    let port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log_path = project.path().join("serve.log");
    let log = fs::File::create(&log_path).unwrap();
    let _server = Server(
        Command::new(cli_bin())
            .args(["serve", "--port", &port.to_string(), "--no-prompt"])
            .current_dir(project.path())
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap(),
    );
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(15);
    let response = loop {
        match http.get(format!("{base}/")).send() {
            Ok(response) => break response,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(err) => panic!(
                "server did not listen: {err}\n{}",
                fs::read_to_string(&log_path).unwrap()
            ),
        }
    };
    let status = response.status();
    let body = response.text().unwrap();
    assert_eq!(
        status,
        200,
        "{body}\n{}",
        fs::read_to_string(&log_path).unwrap()
    );
    assert!(body.contains("<h1>Deka App</h1>"), "{body}");
    let css = http.get(format!("{base}/style.css")).send().unwrap();
    assert_eq!(css.status(), 200);
    assert!(
        css.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("text/css")
    );
    assert!(css.text().unwrap().contains("font-family"));
    for private in ["deka.json", "deka.lock", "serve.log"] {
        assert_eq!(
            http.get(format!("{base}/{private}"))
                .send()
                .unwrap()
                .status(),
            404
        );
    }
}
