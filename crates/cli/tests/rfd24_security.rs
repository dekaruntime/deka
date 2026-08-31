//! Durable HTTP checks for deka#392 — second-pass RFD 24 security claims.
//!
//! These go through `deka serve` and real requests, not generated-source greps.

use base64::Engine;
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

struct Serve {
    child: Child,
    port: u16,
    root: TempDir,
}

impl Serve {
    fn log(&self) -> String {
        fs::read_to_string(self.root.path().join("serve.log")).unwrap_or_default()
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral")
        .local_addr()
        .expect("local addr")
        .port()
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

fn write_security_fixture(dir: &Path, trailing_slash: bool) {
    let serve = if trailing_slash {
        r#""serve": { "mode": "ds", "trailingSlash": true }"#
    } else {
        r#""serve": { "mode": "ds" }"#
    };
    let deka = format!(
        "{{\n  \"name\": \"rfd24-security\",\n  \"type\": \"serve\",\n  {serve},\n  \"tasks\": {{ \"dev\": \"deka serve --dev\" }}\n}}\n"
    );
    fs::write(dir.join("deka.json"), deka).expect("write deka.json");

    fs::create_dir_all(dir.join("app/blog")).expect("mkdir blog");
    fs::create_dir_all(dir.join("app/admin")).expect("mkdir admin");
    fs::create_dir_all(dir.join("api/boom")).expect("mkdir api/boom");
    fs::create_dir_all(dir.join(".cache/dekascript/assets")).expect("mkdir assets");
    fs::write(
        dir.join(".cache/dekascript/assets/ok.js"),
        "export const ok = 1;\n",
    )
    .expect("write ok.js");

    fs::write(
        dir.join("app/blog/page.dsx"),
        "export fn Post() {\n    return <article>blog-secret</article>;\n}\nexport fn Page() {\n    return <main><Post server:defer><span slot=\"fallback\">loading-post</span></Post></main>;\n}\n",
    )
    .expect("write blog page");
    fs::write(
        dir.join("app/admin/page.dsx"),
        "export fn AdminPanel() {\n    return <section>admin-secret</section>;\n}\nexport fn Page() {\n    return <main><AdminPanel server:defer><span slot=\"fallback\">loading-admin</span></AdminPanel></main>;\n}\n",
    )
    .expect("write admin page");
    fs::write(
        dir.join("middleware.ds"),
        r#"export const matcher = ["/_deka/defer"]
interface RequestHeaders { accept: string }
interface ResponseHeaders { location: string }
interface Request { url: string, pathname: string, method: string, headers: RequestHeaders }
interface Response { status: number, body: string, headers: ResponseHeaders }
export fn middleware(request: Request): Option<Response> {
    if (request.headers.accept != "text/x-deka-session") {
        return Some({ status: 401, body: "gated", headers: { location: "" } })
    }
    return None
}
"#,
    )
    .expect("write middleware");
    fs::write(
        dir.join("api/boom/route.ds"),
        r#"interface RequestHeaders { accept: string }
interface Request { url: string, pathname: string, method: string, headers: RequestHeaders }
interface Response { status: number, body: string }
export fn GET(request: Request): Response {
    unsafe { throw new Error("secret:/etc/passwd leaked") }
    return { status: 200, body: "ok" }
}
"#,
    )
    .expect("write boom route");
}

fn spawn_serve(trailing_slash: bool) -> Serve {
    let root = tempfile::tempdir().expect("tempdir");
    init_project(root.path());
    write_security_fixture(root.path(), trailing_slash);
    let port = free_port();
    let log_path = root.path().join("serve.log");
    let log = fs::File::create(&log_path).expect("serve.log");
    let child = Command::new(cli_bin())
        .args(["serve", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root.path())
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka serve");
    let serve = Serve {
        child,
        port,
        root,
    };
    wait_ready(serve.port);
    serve
}

fn client() -> Client {
    Client::builder()
        .redirect(redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client")
}

fn wait_ready(port: u16) {
    let http = client();
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if let Ok(res) = http.get(format!("http://127.0.0.1:{port}/")).send() {
            if res.status().as_u16() == 200 {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!("deka serve did not become ready on port {port}");
}

fn location(res: &reqwest::blocking::Response) -> String {
    res.headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

struct Island {
    name: String,
    id: String,
    mac: String,
}

fn decode_b64(raw: &str) -> String {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .unwrap_or_default();
    String::from_utf8(bytes).unwrap_or_default()
}

fn parse_island(html: &str) -> Island {
    let start = html
        .find("deka-island start:")
        .expect("island start marker");
    let line = &html[start..];
    let end = line.find("-->").unwrap_or(line.len());
    let marker = &line[..end];
    fn field(marker: &str, key: &str) -> String {
        let needle = format!("{key}:");
        let Some(at) = marker.find(&needle) else {
            return String::new();
        };
        let rest = &marker[at + needle.len()..];
        let token = rest.split_whitespace().next().unwrap_or("");
        decode_b64(token)
    }
    Island {
        name: field(marker, "start"),
        id: field(marker, "id"),
        mac: field(marker, "mac"),
    }
}

#[test]
fn rfd24_security_claims_on_http() {
    let serve = spawn_serve(false);
    let http = client();
    let base = format!("http://127.0.0.1:{}", serve.port);

    // 1. /assets absolute-join must 404, not return /etc/passwd.
    let traversal = http
        .get(format!("{base}/assets//etc/passwd"))
        .send()
        .expect("assets traversal request");
    assert_eq!(
        traversal.status().as_u16(),
        404,
        "/assets//etc/passwd must not read the host filesystem, body: {:?}",
        traversal.text().ok()
    );
    let ok_js = http
        .get(format!("{base}/assets/ok.js"))
        .send()
        .expect("assets ok");
    assert_eq!(
        ok_js.status().as_u16(),
        200,
        "legitimate cache assets should still serve"
    );

    // 2. trailingSlash: false — GET //evil.com/foo/ must not protocol-relative redirect.
    let open = http
        .get(format!("{base}//evil.com/foo/"))
        .send()
        .expect("open redirect request");
    let loc = location(&open);
    assert!(
        !loc.starts_with("//") && !loc.contains("://evil.com"),
        "trailingSlash=false must not yield off-origin Location, got {loc:?} status {}",
        open.status()
    );

    // 4. middleware must run on POST /_deka/defer and short-circuit.
    let gated = http
        .post(format!("{base}/_deka/defer"))
        .header("content-type", "application/json")
        .body(r#"{"islands":[]}"#)
        .send()
        .expect("defer without session");
    assert_eq!(
        gated.status().as_u16(),
        401,
        "middleware must reject unauthenticated defer, body: {:?}\nserve.log:\n{}",
        gated.text().ok(),
        serve.log()
    );

    // 5. Throwing handler must not leak the exception message.
    let boom = http
        .get(format!("{base}/api/boom"))
        .send()
        .expect("GET /api/boom");
    let boom_status = boom.status().as_u16();
    let boom_body = boom.text().expect("boom body");
    assert!(
        !boom_body.contains("secret:/etc/passwd") && !boom_body.contains("/etc/passwd"),
        "handler error must not leak the exception, status {boom_status} body {boom_body}"
    );
    let api_entry = fs::read_to_string(
        serve
            .root
            .path()
            .join(".cache/dekascript/api-entry.ds"),
    )
    .unwrap_or_default();
    assert!(
        api_entry.contains("runApiRouter"),
        "API entry should dispatch through ui/router: {api_entry}"
    );
    assert!(
        !api_entry.contains("e.message"),
        "generated API wrapper must not return e.message: {api_entry}"
    );
    assert!(
        include_str!("../../deka_ui/js/router.js").contains("Internal Server Error")
            && !include_str!("../../deka_ui/js/router.js").contains("e.message"),
        "ui/router must use an opaque 500"
    );
}

#[test]
fn rfd24_defer_entitlement() {
    let serve = spawn_serve(false);
    let http = client();
    let base = format!("http://127.0.0.1:{}", serve.port);
    rfd24_defer_entitlement_inner(&http, &base, &serve);
}

fn rfd24_defer_entitlement_inner(http: &Client, base: &str, serve: &Serve) {
    let empty = http
        .post(format!("{base}/_deka/defer"))
        .header("content-type", "application/json")
        .header("accept", "text/x-deka-session")
        .body(r#"{"islands":[]}"#)
        .send()
        .expect("empty defer");
    let empty_status = empty.status().as_u16();
    let empty_body = empty.text().expect("empty body");
    assert_eq!(
        empty_status, 200,
        "authed empty defer batch should 200, body: {empty_body}\nserve.log:\n{}\ndefer-entry:\n{}",
        serve.log(),
        fs::read_to_string(serve.root.path().join(".cache/dekascript/defer-entry.dsx"))
            .unwrap_or_default()
    );

    let blog = http
        .get(format!("{base}/blog"))
        .send()
        .expect("GET /blog")
        .text()
        .expect("blog html");
    assert!(
        blog.contains("loading-post"),
        "blog should render the defer fallback: {blog}"
    );
    let post = parse_island(&blog);
    assert_eq!(post.name, "Post");
    let cross = http
        .post(format!("{base}/_deka/defer"))
        .header("content-type", "application/json")
        .header("accept", "text/x-deka-session")
        .body(
            serde_json::json!({
                "islands": [{
                    "id": post.id,
                    "name": "AdminPanel",
                    "props": {},
                    "mac": post.mac,
                }]
            })
            .to_string(),
        )
        .send()
        .expect("cross-page defer");
    let cross_status = cross.status().as_u16();
    let cross_body = cross.text().expect("cross body");
    let defer_src = fs::read_to_string(
        serve
            .root
            .path()
            .join(".cache/dekascript/defer-entry.dsx"),
    )
    .unwrap_or_default();
    assert_eq!(
        cross_status, 200,
        "authed defer should 200, body: {cross_body}\nserve.log:\n{}\ndefer-entry:\n{defer_src}",
        serve.log()
    );
    assert!(
        !cross_body.contains("admin-secret"),
        "signed Post island must not render AdminPanel: {cross_body}"
    );

    let admin = http
        .get(format!("{base}/admin"))
        .send()
        .expect("GET /admin")
        .text()
        .expect("admin html");
    let panel = parse_island(&admin);
    assert_eq!(panel.name, "AdminPanel");
    let legit = http
        .post(format!("{base}/_deka/defer"))
        .header("content-type", "application/json")
        .header("accept", "text/x-deka-session")
        .body(
            serde_json::json!({
                "islands": [{
                    "id": panel.id,
                    "name": panel.name,
                    "props": {},
                    "mac": panel.mac,
                }]
            })
            .to_string(),
        )
        .send()
        .expect("legit defer");
    let legit_body = legit.text().expect("legit body");
    assert!(
        legit_body.contains("admin-secret"),
        "valid AdminPanel MAC should render the island: {legit_body}"
    );
}

#[test]
fn rfd24_open_redirect_trailing_slash_true() {
    let serve = spawn_serve(true);
    let http = client();
    let base = format!("http://127.0.0.1:{}", serve.port);
    let open = http
        .get(format!("{base}//evil.com/foo"))
        .send()
        .expect("open redirect request");
    let loc = location(&open);
    assert!(
        !loc.starts_with("//") && !loc.contains("://evil.com"),
        "trailingSlash=true must not yield off-origin Location, got {loc:?} status {}",
        open.status()
    );
}
