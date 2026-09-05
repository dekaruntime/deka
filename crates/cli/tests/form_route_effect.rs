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

const FORM_ACTION: &str = "/api/cart";
const FORM_METHOD: &str = "post";
const FORM_ENCTYPE: &str = "application/x-www-form-urlencoded";
const FIELD_NAME: &str = "sku";
const FIELD_VALUE: &str = "sku-42";

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
    let page = format!(
        r#"import {{ Form }} from "ui/form"

export fn Page() {{
    return <Form action="{FORM_ACTION}" method="{FORM_METHOD}"><input name="{FIELD_NAME}" value="{FIELD_VALUE}" /><button>Add</button></Form>
}}
"#
    );
    fs::write(root.join("app/page.dsx"), page).expect("write page");

    let route = root.join(FORM_ACTION.trim_start_matches('/'));
    fs::create_dir_all(&route).expect("create api route directory");
    fs::write(
        route.join("route.ds"),
        r#"interface Request { body: string }
interface Response { status: number, body: string }

export fn POST(request: Request) Response {
    return { status: 200, body: request.body }
}
"#,
    )
    .expect("write cart route");
}

fn form_attribute(html: &str, name: &str) -> String {
    let form_start = html.find("<form ").expect("rendered form start");
    let form_end = html[form_start..]
        .find('>')
        .map(|offset| form_start + offset)
        .expect("rendered form end");
    let form_tag = &html[form_start..=form_end];
    let prefix = format!("{name}=\"");
    let Some(value_start) = form_tag.find(&prefix) else {
        return String::new();
    };
    let value_start = value_start + prefix.len();
    let value_end = form_tag[value_start..]
        .find('"')
        .map(|offset| value_start + offset)
        .expect("form attribute value end");
    form_tag[value_start..value_end].to_string()
}

fn input_attribute(html: &str, name: &str) -> String {
    let input_start = html.find("<input ").expect("rendered input start");
    let input_end = html[input_start..]
        .find('>')
        .map(|offset| input_start + offset)
        .expect("rendered input end");
    let input_tag = &html[input_start..=input_end];
    let prefix = format!("{name}=\"");
    let value_start = input_tag.find(&prefix).expect("input attribute");
    let value_start = value_start + prefix.len();
    let value_end = input_tag[value_start..]
        .find('"')
        .map(|offset| value_start + offset)
        .expect("input attribute value end");
    input_tag[value_start..value_end].to_string()
}

fn wait_ready(client: &Client, port: u16) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(format!("http://127.0.0.1:{port}/")).send() {
            if response.status().as_u16() == 200 {
                return;
            }
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
    let action = form_attribute(&page, "action");
    let method = form_attribute(&page, "method");
    let enctype = form_attribute(&page, "enctype");
    let field_name = input_attribute(&page, "name");
    let field_value = input_attribute(&page, "value");
    let enctype = if enctype.is_empty() {
        FORM_ENCTYPE.to_string()
    } else {
        enctype
    };

    let response = client
        .request(
            method
                .to_ascii_uppercase()
                .parse()
                .expect("rendered form method"),
            format!("http://127.0.0.1:{port}{action}"),
        )
        .header("content-type", enctype)
        .form(&[(field_name.as_str(), field_value.as_str())])
        .send()
        .expect("submit rendered form to cart route");
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.text().expect("read route response"), "sku=sku-42");
}
