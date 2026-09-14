//! The default project must be a working DekaScript app-router tree
//! (RFD 24) that `deka serve` can render without extra setup.

use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn printed_line_count(output: &std::process::Output) -> usize {
    // stdio::raw writes to stderr; stdout belongs to the program.
    String::from_utf8_lossy(&output.stderr)
        .trim_end_matches('\n')
        .lines()
        .count()
}

fn assert_scaffold(root: &Path) {
    assert!(
        root.join("app/page.dsx").is_file(),
        "init must write app/page.dsx"
    );
    assert!(
        root.join("app/layout.dsx").is_file(),
        "init must write app/layout.dsx"
    );
    assert!(
        root.join("src/ui/Counter.dsx").is_file(),
        "init must write src/ui/Counter.dsx"
    );
    assert!(
        root.join("index.html").is_file(),
        "init must write root index.html"
    );
    assert!(
        !root.join("public/index.html").exists(),
        "public/index.html collides with the root document"
    );
    assert!(
        !root.join("api").exists(),
        "init must not create an empty api/ directory"
    );
    assert!(root.join("public/style.css").is_file());
    assert!(root.join(".gitignore").is_file());

    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("deka.json")).unwrap()).unwrap();
    assert_eq!(config["type"], "serve");
    assert_eq!(config["serve"]["mode"], "ds");
    assert!(config["serve"].get("entry").is_none());
    assert_eq!(config["tasks"]["dev"], "deka serve --dev");

    let page = fs::read_to_string(root.join("app/page.dsx")).unwrap();
    assert!(page.contains("export fn Page()"), "{page}");
    assert!(page.contains("<h1>Deka App</h1>"), "{page}");
    assert!(page.contains("client:load"), "{page}");
    assert!(page.contains("fn greeting("), "{page}");

    let counter = fs::read_to_string(root.join("src/ui/Counter.dsx")).unwrap();
    assert!(counter.contains("useState"), "{counter}");

    let index = fs::read_to_string(root.join("index.html")).unwrap();
    assert!(index.contains("<!--deka-app-->"), "{index}");
    assert!(index.contains("<!--deka-head-->"), "{index}");
    assert!(index.contains("<!--deka-scripts-->"), "{index}");

    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(gitignore.contains("ds_modules/"), "{gitignore}");
    assert!(gitignore.contains("dist/"), "{gitignore}");
    assert!(gitignore.contains(".deka.json-backup-*"), "{gitignore}");
    assert!(gitignore.contains(".deka.lock-backup-*"), "{gitignore}");
}

fn wire_dsc(command: &mut Command) {
    if let Ok(dsc) = std::env::var("DEKA_DSC") {
        command.env("DEKA_DSC", dsc);
    } else if let Some(dsc) = compiler::dsc::find_dsc().ok().flatten() {
        command.env("DEKA_DSC", dsc);
    }
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
fn init_creates_dekascript_app_and_preserves_existing_files() {
    let project = tempfile::tempdir().expect("tempdir");
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(project.path())
        .output()
        .expect("run deka init");
    assert!(
        output.status.success(),
        "deka init must succeed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let printed = String::from_utf8_lossy(&output.stderr);
    assert!(
        printed_line_count(&output) <= 3,
        "init output must be at most 3 lines, got:\n{printed}"
    );
    assert!(
        printed.contains("deka serve"),
        "init must tell the user to run deka serve:\n{printed}"
    );

    assert_scaffold(project.path());

    let page = project.path().join("app/page.dsx");
    fs::write(&page, "user-authored page").unwrap();
    assert!(Command::new(cli_bin())
        .arg("init")
        .current_dir(project.path())
        .status()
        .unwrap()
        .success());
    assert_eq!(fs::read_to_string(page).unwrap(), "user-authored page");
}

#[test]
fn fresh_init_compiles_the_page() {
    let project = tempfile::tempdir().unwrap();
    assert!(Command::new(cli_bin())
        .arg("init")
        .current_dir(project.path())
        .status()
        .unwrap()
        .success());

    let mut check = Command::new(cli_bin());
    check
        .args(["check", "app/page.dsx"])
        .current_dir(project.path());
    wire_dsc(&mut check);
    let output = check.output().expect("run deka check");
    assert!(
        output.status.success(),
        "scaffolded app/page.dsx must typecheck: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
    assert!(Command::new(cli_bin())
        .arg("init")
        .current_dir(project.path())
        .status()
        .unwrap()
        .success());
    let port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log_path = project.path().join("serve.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut serve = Command::new(cli_bin());
    serve
        .args(["serve", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(project.path())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log));
    wire_dsc(&mut serve);
    let _server = Server(serve.spawn().unwrap());
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last = String::new();
    let body = loop {
        match http.get(format!("{base}/")).send() {
            Ok(response) if response.status().as_u16() == 200 => {
                break response.text().unwrap();
            }
            Ok(response) => {
                last = format!(
                    "status {} {}",
                    response.status(),
                    response.text().unwrap_or_default()
                );
                if Instant::now() >= deadline {
                    panic!(
                        "server did not serve 200: {last}\n{}",
                        fs::read_to_string(&log_path).unwrap()
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(err) => panic!(
                "server did not listen: {err}\n{last}\n{}",
                fs::read_to_string(&log_path).unwrap()
            ),
        }
    };
    assert!(body.contains("Deka App"), "{body}");
    assert!(body.contains("Hello, World."), "{body}");
    assert!(
        body.contains("id=\"counter\"") && body.contains("data-deka-island=\"Counter\""),
        "served page must include the counter island:\n{body}"
    );
    let css = http.get(format!("{base}/style.css")).send().unwrap();
    assert_eq!(css.status(), 200);
    assert!(css.headers()["content-type"]
        .to_str()
        .unwrap()
        .contains("text/css"));
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
