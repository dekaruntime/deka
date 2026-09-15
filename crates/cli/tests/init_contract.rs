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
        root.join("app/Counter.dsx").is_file(),
        "init must write app/Counter.dsx"
    );
    assert!(
        !root.join("src").exists(),
        "init must not create a src/ directory (RFD 24 tree has none): deka#1044"
    );
    assert!(
        !root.join("app/not-found.dsx").exists(),
        "the default 404 must be a static public/404.html, not a rendered route: deka#1045"
    );
    assert!(
        root.join("public/404.html").is_file(),
        "init must write public/404.html"
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
    assert!(
        root.join("public/favicon.ico").is_file(),
        "init must write a default favicon: deka#1066"
    );
    assert!(
        !root.join(".cache").exists(),
        "init must not create a top-level .cache — caching lives under ds_modules only: deka#1065"
    );

    // deka#1046 / deka#1000: the scaffolded style.css must be readable,
    // properly formatted CSS, not a single escaped line.
    let style = fs::read_to_string(root.join("public/style.css")).unwrap();
    assert!(
        style.lines().filter(|l| !l.trim().is_empty()).count() > 1,
        "style.css must be formatted with more than one line:\n{style}"
    );

    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("deka.json")).unwrap()).unwrap();
    assert_eq!(config["type"], "serve");
    assert_eq!(config["serve"]["mode"], "ds");
    assert!(config["serve"].get("entry").is_none());
    assert_eq!(config["tasks"]["dev"], "deka serve --dev");

    // deka#973: the scaffold declares RFD-53 phase-aware permissions, not the
    // legacy `security` shape — dev gets a working grant with zero manual
    // editing, prod stays fully denied (declared, not implicit).
    assert!(
        config.get("security").is_none(),
        "scaffold must not use the legacy security shape: {config}"
    );
    assert_eq!(config["permissions"]["dev"]["read"], true);
    assert!(config["permissions"]["dev"]["write"].is_array());
    assert_eq!(config["permissions"]["dev"]["wasm"], true);
    assert_eq!(
        config["permissions"]["prod"],
        serde_json::json!({}),
        "prod permissions must be declared and empty (fully denied)"
    );

    let page = fs::read_to_string(root.join("app/page.dsx")).unwrap();
    assert!(page.contains("export fn Page()"), "{page}");
    assert!(page.contains("<h1>Deka App</h1>"), "{page}");
    assert!(page.contains("client:load"), "{page}");
    assert!(page.contains("fn greeting("), "{page}");

    // deka#999: the scaffold must teach the idiomatic destructured tuple
    // form (`const [n, setN] = useState(0)`), matching docs/dekascript/
    // fast-refresh.mdx, not the index-access workaround
    // (`const pair = useState(0); const n = pair[0]`) that implies
    // destructuring does not work.
    let counter = fs::read_to_string(root.join("app/Counter.dsx")).unwrap();
    assert!(counter.contains("useState"), "{counter}");
    assert!(
        counter.contains("const [n, setN] = useState(0)"),
        "scaffold must teach destructured useState, not an index-access workaround:\n{counter}"
    );
    assert!(
        !counter.contains("pair[0]") && !counter.contains("pair[1]"),
        "scaffold must not fall back to index-access on the useState tuple:\n{counter}"
    );
    // deka#999: id="counter" existed only so tests could find the element;
    // a generated user project should not carry our test scaffolding.
    assert!(
        !counter.contains("id=\"counter\""),
        "scaffold must not carry test-only id attributes:\n{counter}"
    );

    let index = fs::read_to_string(root.join("index.html")).unwrap();
    // deka#1047: the scaffold is a clean, Vite-shaped document — no hole
    // markers, no explicit entry <script>. The split points (</head>,
    // <div id="app">, before </body>) are inferred structurally at SSR time.
    assert!(!index.contains("<!--deka-app-->"), "{index}");
    assert!(!index.contains("<!--deka-head-->"), "{index}");
    assert!(!index.contains("<!--deka-scripts-->"), "{index}");
    assert!(!index.contains("<script"), "{index}");
    assert!(index.contains("<div id=\"app\"></div>"), "{index}");
    assert!(
        index.contains("rel=\"icon\"") && index.contains("href=\"/favicon.ico\""),
        "index.html must link the default favicon (deka#1066): {index}"
    );

    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(gitignore.contains("ds_modules/"), "{gitignore}");
    assert!(gitignore.contains("dist/"), "{gitignore}");
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
        printed.contains("deka dev"),
        "init must tell the user to run deka dev: deka#1064\n{printed}"
    );
    assert!(
        !printed.contains("deka serve"),
        "init must recommend deka dev, not deka serve: deka#1064\n{printed}"
    );
    assert!(
        printed.contains("Next steps:"),
        "init's closing block must read as instructions under a Next steps heading: deka#1064\n{printed}"
    );

    // deka#1042: the ASCII banner (restored to stdio in #698) must appear on
    // the init path too, not just `deka dev`/`serve`.
    assert!(
        printed_line_count(&output) > 10,
        "init with a banner + one progress line per file should print more than a bare summary, got:\n{printed}"
    );

    // deka#1043: one aligned `[create] <path>` progress line per meaningful
    // action, so the user can see what was written.
    for expected in [
        "[create] deka.json",
        "[create] app/page.dsx",
        "[create] app/Counter.dsx",
        "[create] public/404.html",
        "[create] public/favicon.ico",
    ] {
        assert!(
            printed.contains(expected),
            "init must print progress line {expected:?}, got:\n{printed}"
        );
    }

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
        body.contains("data-deka-island=\"Counter\""),
        "served page must include the counter island:\n{body}"
    );
    let css = http.get(format!("{base}/style.css")).send().unwrap();
    assert_eq!(css.status(), 200);
    assert!(css.headers()["content-type"]
        .to_str()
        .unwrap()
        .contains("text/css"));
    assert!(css.text().unwrap().contains("font-family"));

    // deka#1045: an unmatched route must serve the static public/404.html,
    // not the JS-rendered app-router fallback.
    let missing = http
        .get(format!("{base}/this-route-does-not-exist"))
        .send()
        .unwrap();
    assert_eq!(missing.status(), 404);
    assert!(missing.headers()["content-type"]
        .to_str()
        .unwrap()
        .contains("text/html"));
    let missing_body = missing.text().unwrap();
    assert!(
        missing_body.contains("Not found"),
        "404 body must come from public/404.html:\n{missing_body}"
    );

    for private in ["deka.json", "deka.lock", "serve.log"] {
        assert_eq!(
            http.get(format!("{base}/{private}"))
                .send()
                .unwrap()
                .status(),
            404
        );
    }

    assert!(
        !project.path().join(".cache").exists(),
        "deka serve must not create a top-level .cache — caching lives under ds_modules only: deka#1065"
    );
}

/// deka#973: `deka dev` on a freshly `deka init`'d project must serve HTTP
/// 200 with zero manual `deka.json` editing — the scaffold's declared
/// `permissions.dev` block (RFD 53) is what makes this work, not an
/// undocumented runtime special case.
#[test]
fn fresh_init_dev_serves_without_manual_permission_edits() {
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
    let log_path = project.path().join("dev.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut dev = Command::new(cli_bin());
    dev.args(["dev", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(project.path())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log));
    wire_dsc(&mut dev);
    let _server = Server(dev.spawn().unwrap());

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
                        "deka dev did not serve 200: {last}\n{}",
                        fs::read_to_string(&log_path).unwrap()
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(err) => panic!(
                "deka dev did not listen: {err}\n{last}\n{}",
                fs::read_to_string(&log_path).unwrap()
            ),
        }
    };
    assert!(body.contains("Deka App"), "{body}");
    assert!(body.contains("Hello, World."), "{body}");

    let log_contents = fs::read_to_string(&log_path).unwrap();
    assert!(
        !log_contents.to_ascii_lowercase().contains("permission denied")
            && !log_contents.contains("invalid security policy")
            && !log_contents.contains("invalid permissions"),
        "deka dev must not hit the permission wall on a fresh scaffold:\n{log_contents}"
    );

    assert!(
        !project.path().join(".cache").exists(),
        "deka dev must not create a top-level .cache — caching lives under ds_modules only: deka#1065"
    );
}

/// deka#973: a project with NO declared permissions at all (predating RFD 53,
/// or a manifest edited by hand) still falls back to implicit dev defaults —
/// but that widening must never be silent. It has to say what it granted and
/// how to make it an explicit `permissions.dev` block.
#[test]
fn dev_without_declared_permissions_prints_an_implicit_grant_notice() {
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
    // Strip the scaffold's declared permissions to simulate a pre-RFD-53
    // manifest that never went through `deka init` with this fix.
    fs::write(
        project.path().join("deka.json"),
        r#"{ "name": "legacy", "type": "serve", "serve": { "mode": "ds" } }"#,
    )
    .unwrap();

    let port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log_path = project.path().join("dev.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut dev = Command::new(cli_bin());
    dev.args(["dev", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(project.path())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log));
    wire_dsc(&mut dev);
    let _server = Server(dev.spawn().unwrap());

    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match http.get(format!("{base}/")).send() {
            Ok(response) if response.status().as_u16() == 200 => break,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => panic!(
                "deka dev did not serve 200: {}",
                fs::read_to_string(&log_path).unwrap()
            ),
        }
    }

    let log_contents = fs::read_to_string(&log_path).unwrap();
    assert!(
        log_contents.contains("implicitly granted") && log_contents.contains("permissions.dev"),
        "the implicit dev-default grant must be announced, not silent:\n{log_contents}"
    );
}
