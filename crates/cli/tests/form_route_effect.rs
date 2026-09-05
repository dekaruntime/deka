//! RFD 24 §5.4: a plain form must be able to submit to an api route.
//!
//! This intentionally exercises the rendered HTML and a real HTTP POST. It
//! does not test that a symbol named `Form` exists in isolation: removing the
//! Form wiring from the page must make the rendered-form assertion fail.

use reqwest::blocking::Client;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

struct Serve {
    child: Child,
    _root: TempDir,
}

impl Drop for Serve {
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

fn init_project(root: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(root)
        .output()
        .expect("run deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_fixture(root: &Path) {
    fs::write(
        root.join("app/page.dsx"),
        r#"import { Form } from "ui/form"

export fn Page() {
    return <Form action="/api/cart" method="post"><input name="sku" value="sku-42" /><button>Add</button></Form>
}
"#,
    )
    .expect("write page");

    fs::create_dir_all(root.join("api/cart")).expect("create api route directory");
    fs::write(
        root.join("api/cart/route.ds"),
        r#"interface Request { body: string }
interface Response { status: number, body: string }

export fn POST(request: Request) Response {
    return { status: 200, body: request.body }
}
"#,
    )
    .expect("write cart route");
}

fn wait_ready(client: &Client, port: u16) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(format!("http://127.0.0.1:{port}/")).send()
            && response.status().as_u16() == 200
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!("deka serve did not become ready on port {port}");
}

#[test]
fn form_posts_to_route_and_route_receives_submitted_fields() {
    let root = tempfile::tempdir().expect("create project directory");
    init_project(root.path());
    write_fixture(root.path());

    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("build HTTP client");
    let port = free_port();
    let child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root.path())
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn deka serve");
    let _serve = Serve { child, _root: root };
    wait_ready(&client, port);

    let page = client
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .expect("GET page")
        .text()
        .expect("read page HTML");
    assert!(
        page.contains(r#"<form action="/api/cart" method="post">"#),
        "Form must render a browser-native form pointing at the route: {page}"
    );
    assert!(
        page.contains(r#"name="sku" value="sku-42""#),
        "form fields must survive rendering: {page}"
    );

    let response = client
        .post(format!("http://127.0.0.1:{port}/api/cart"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("sku=sku-42")
        .send()
        .expect("submit rendered form to cart route");
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.text().expect("read route response"), "sku=sku-42");
}
