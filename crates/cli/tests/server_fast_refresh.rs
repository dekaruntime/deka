#![cfg(feature = "dev-server")]

//! deka#956: server-rendered Fast Refresh morphs HTML without a full reload,
//! preserves hydrated island state and form values, and full-reloads when the
//! island module itself changes. Real topology: spawned `deka dev` + Chromium CDP.

use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tungstenite::protocol::WebSocket;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn fixture_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/server-fast-refresh")
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

struct ChromeProcess(Child);

impl Drop for ChromeProcess {
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

fn spawn_dev(root: &Path, port: u16) -> ServeProcess {
    let log_path = root.join("dev.log");
    let log = fs::File::create(&log_path).expect("create dev log");
    let mut command = Command::new(cli_bin());
    command
        .args(["dev", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy");
    if let Ok(dsc) = std::env::var("DEKA_DSC") {
        command.env("DEKA_DSC", dsc);
    } else if let Some(dsc) = compiler::dsc::find_dsc().ok().flatten() {
        command.env("DEKA_DSC", dsc);
    } else {
        let dsc_beside_cli = Path::new(cli_bin()).with_file_name("dsc");
        if dsc_beside_cli.is_file() {
            command.env("DEKA_DSC", dsc_beside_cli);
        }
    }
    command
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log));
    ServeProcess {
        child: command.spawn().expect("spawn deka dev"),
        log_path,
    }
}

fn wait_body(client: &Client, url: &str, log_path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(90);
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
    panic!("deka dev did not become ready: {last}\ndev.log:\n{log}");
}

fn set_timeout(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, dur: Duration) {
    if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
        let _ = stream.set_read_timeout(Some(dur));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    }
}

fn connect_hmr(port: u16) -> WebSocket<MaybeTlsStream<TcpStream>> {
    let url = format!("ws://127.0.0.1:{port}/_deka/hmr");
    let (mut ws, _) = connect(&url).unwrap_or_else(|err| panic!("hmr ws {url}: {err}"));
    set_timeout(&mut ws, Duration::from_millis(250));
    ws.send(Message::Text(r#"{"type":"subscribe","path":"/"}"#.into()))
        .expect("subscribe");
    ws
}

fn wait_hmr_message(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + timeout;
    let mut last = String::new();
    while Instant::now() < deadline {
        match ws.read() {
            Ok(Message::Text(text)) => {
                last = text.to_string();
                if let Ok(value) = serde_json::from_str::<Value>(&last) {
                    if pred(&value) {
                        return value;
                    }
                }
            }
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    panic!("timed out waiting for HMR message; last={last}");
}

fn chrome_bin() -> PathBuf {
    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium-browser",
        "/usr/bin/chromium",
        "/usr/bin/google-chrome-stable",
    ];
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return path;
        }
    }
    panic!("Chromium/Chrome not found for CDP e2e (deka#956)");
}

struct Cdp {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    navigations: u32,
}

impl Cdp {
    fn connect(debug_port: u16) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .no_proxy()
            .build()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut ws_url = String::new();
        while Instant::now() < deadline {
            if let Ok(res) = client
                .put(format!("http://127.0.0.1:{debug_port}/json/new?about:blank"))
                .send()
            {
                if let Ok(body) = res.text() {
                    if let Ok(value) = serde_json::from_str::<Value>(&body) {
                        if let Some(url) = value
                            .get("webSocketDebuggerUrl")
                            .and_then(|v| v.as_str())
                        {
                            ws_url = url.to_string();
                            break;
                        }
                    }
                }
            }
            if let Ok(res) = client
                .get(format!("http://127.0.0.1:{debug_port}/json/list"))
                .send()
            {
                if let Ok(body) = res.text() {
                    if let Ok(list) = serde_json::from_str::<Vec<Value>>(&body) {
                        if let Some(url) = list.iter().find_map(|page| {
                            page.get("webSocketDebuggerUrl")
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        }) {
                            ws_url = url;
                            break;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!ws_url.is_empty(), "chrome never exposed a page websocket");
        let (mut ws, _) = connect(&ws_url).expect("cdp websocket");
        set_timeout(&mut ws, Duration::from_millis(250));
        let mut cdp = Self {
            ws,
            next_id: 1,
            navigations: 0,
        };
        cdp.call("Page.enable", json!({}));
        cdp.call("Runtime.enable", json!({}));
        cdp.call("Page.setLifecycleEventsEnabled", json!({ "enabled": true }));
        cdp
    }

    fn drain(&mut self) {
        loop {
            match self.ws.read() {
                Ok(Message::Text(text)) => self.on_event(&text),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    fn on_event(&mut self, text: &str) {
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            if value.get("method").and_then(|v| v.as_str()) == Some("Page.frameNavigated") {
                let is_main = value
                    .pointer("/params/frame/parentId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.is_empty())
                    .unwrap_or(true);
                if is_main {
                    self.navigations += 1;
                }
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({ "id": id, "method": method, "params": params });
        self.ws
            .send(Message::Text(payload.to_string().into()))
            .expect("cdp send");
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match self.ws.read() {
                Ok(Message::Text(text)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                        if value.get("id").and_then(|v| v.as_u64()) == Some(id) {
                            return value;
                        }
                        self.on_event(&text);
                    }
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        panic!("cdp {method} timed out");
    }

    fn navigate(&mut self, url: &str) {
        self.call("Page.navigate", json!({ "url": url }));
        let deadline = Instant::now() + Duration::from_secs(45);
        while Instant::now() < deadline {
            match self.ws.read() {
                Ok(Message::Text(text)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                        let method = value.get("method").and_then(|v| v.as_str());
                        if method == Some("Page.loadEventFired")
                            || method == Some("Page.lifecycleEvent")
                                && value
                                    .pointer("/params/name")
                                    .and_then(|v| v.as_str())
                                    == Some("load")
                        {
                            self.on_event(&text);
                            return;
                        }
                        self.on_event(&text);
                    }
                }
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        panic!("page load timed out");
    }

    fn eval(&mut self, expression: &str) -> Value {
        let result = self.call(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
            }),
        );
        if result.pointer("/result/exceptionDetails").is_some() {
            panic!("cdp eval failed: {result} ({expression})");
        }
        result
            .pointer("/result/result/value")
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn eval_string(&mut self, expression: &str) -> String {
        match self.eval(expression) {
            Value::String(s) => s,
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            other => other.to_string(),
        }
    }

    fn wait_eval_eq(&mut self, expression: &str, expected: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        let mut last = String::new();
        while Instant::now() < deadline {
            last = self.eval_string(expression);
            if last == expected {
                return last;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {expression} == {expected:?}; last={last}");
    }
}

fn spawn_chrome(debug_port: u16, profile: &Path) -> ChromeProcess {
    fs::create_dir_all(profile).expect("chrome profile");
    let bin = chrome_bin();
    let child = Command::new(&bin)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-extensions",
            "--disable-popup-blocking",
            "--remote-allow-origins=*",
            &format!("--remote-debugging-port={debug_port}"),
            &format!("--user-data-dir={}", profile.display()),
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn chrome {}: {err}", bin.display()));
    ChromeProcess(child)
}

#[test]
fn hmr_payload_html_update_then_island_reload() {
    let root = tempfile::tempdir().expect("temp project");
    copy_tree(&fixture_src(), root.path());
    let port = free_port();
    let server = spawn_dev(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .no_proxy()
        .build()
        .expect("http client");
    let url = format!("http://127.0.0.1:{port}/");
    let body = wait_body(&client, &url, &server.log_path);
    assert!(
        body.contains("hello server") && body.contains("__deka_hmr_client"),
        "dev HTML must inject HMR and SSR the page:\n{body}"
    );
    assert!(
        body.contains("data-deka-island=\"Counter\"") || body.contains("id=\"counter\""),
        "SSR must emit the counter island:\n{body}"
    );

    let mut ws = connect_hmr(port);
    fs::write(
        root.path().join("app/page.dsx"),
        r#"export fn Page() ReactNode {
  return <section>
      <h1 id="server-title">hello refreshed</h1>
      <form>
        <input id="outside-input" name="note" value="initial" />
      </form>
      <div id="spacer" class="spacer"></div>
    </section>
}
"#,
    )
    .unwrap();
    let update = wait_hmr_message(&mut ws, Duration::from_secs(45), |value| {
        value.get("type").and_then(|v| v.as_str()) == Some("html-update")
            && value
                .get("html")
                .and_then(|v| v.as_str())
                .is_some_and(|html| html.contains("hello refreshed"))
    });
    assert_eq!(update["selector"], "#app");

    fs::write(
        root.path().join("src/ui/Counter.dsx"),
        r#"export fn Counter() ReactNode {
  const pair = useState(10)
  const n = pair[0]
  const setN = pair[1]
  return <button type="button" id="counter" onClick={fn() void {
      setN(n + 1)
    }}>{n}</button>
}
"#,
    )
    .unwrap();
    let reload = wait_hmr_message(&mut ws, Duration::from_secs(45), |value| {
        value.get("type").and_then(|v| v.as_str()) == Some("reload")
            && value.get("reason").and_then(|v| v.as_str()) == Some("island-source")
    });
    assert_eq!(reload["type"], "reload");
}

/// deka#956 reopened: the merged fix (e46c57bd) only ever exercised edits to
/// `app/page.dsx`; `app/layout.dsx` — a server component just like the page —
/// went untested. Assert the root layout gets the same html-update morph
/// treatment, not a full reload.
#[test]
fn hmr_payload_layout_edit_morphs_without_full_reload() {
    let root = tempfile::tempdir().expect("temp project");
    copy_tree(&fixture_src(), root.path());
    let port = free_port();
    let server = spawn_dev(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .no_proxy()
        .build()
        .expect("http client");
    let url = format!("http://127.0.0.1:{port}/");
    let body = wait_body(&client, &url, &server.log_path);
    assert!(body.contains("hello server"), "expected SSR page:\n{body}");

    let mut ws = connect_hmr(port);
    fs::write(
        root.path().join("app/layout.dsx"),
        r#"import { Counter } from "../src/ui/Counter.dsx"

interface LayoutProps {
  children: ReactNode
}

export fn Layout(props: LayoutProps) ReactNode {
  return <div>
      <Counter client:load />
      <main id="layout-marker" data-refreshed="yes">{props.children}</main>
    </div>
}
"#,
    )
    .unwrap();

    let message = wait_hmr_message(&mut ws, Duration::from_secs(45), |value| {
        let ty = value.get("type").and_then(|v| v.as_str());
        ty == Some("html-update") || ty == Some("reload")
    });
    assert_eq!(
        message["type"], "html-update",
        "editing app/layout.dsx must morph like any other server component, not full-reload; \
         got {message:?}\ndev.log:\n{}",
        fs::read_to_string(&server.log_path).unwrap_or_default()
    );
    assert!(
        message["html"]
            .as_str()
            .is_some_and(|html| html.contains("layout-marker")),
        "html-update payload must carry the refreshed layout markup: {message:?}"
    );
}

/// deka#1048: editing the root `index.html` document shell must not be a
/// silent no-op. The shell lives outside `#app`, so the existing #app-only
/// morph can never reflect it — the correct behavior is the same full-reload
/// fallback already used for island-module edits, not doing nothing.
#[test]
fn hmr_payload_index_html_edit_triggers_reload() {
    let root = tempfile::tempdir().expect("temp project");
    copy_tree(&fixture_src(), root.path());
    let port = free_port();
    let server = spawn_dev(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .no_proxy()
        .build()
        .expect("http client");
    let url = format!("http://127.0.0.1:{port}/");
    let _ = wait_body(&client, &url, &server.log_path);

    let mut ws = connect_hmr(port);
    let index_html = fs::read_to_string(root.path().join("index.html")).unwrap();
    fs::write(
        root.path().join("index.html"),
        index_html.replace("<title>Server Fast Refresh</title>", "<title>Edited Shell</title>"),
    )
    .unwrap();

    let message = wait_hmr_message(&mut ws, Duration::from_secs(45), |value| {
        value.get("type").and_then(|v| v.as_str()) == Some("reload")
    });
    assert_eq!(
        message["type"], "reload",
        "editing index.html must trigger a full reload rather than a no-op; \
         dev.log:\n{}",
        fs::read_to_string(&server.log_path).unwrap_or_default()
    );
}

#[test]
fn cdp_morph_preserves_island_state_and_island_edit_reloads() {
    let root = tempfile::tempdir().expect("temp project");
    copy_tree(&fixture_src(), root.path());
    let port = free_port();
    let server = spawn_dev(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .no_proxy()
        .build()
        .expect("http client");
    let url = format!("http://127.0.0.1:{port}/");
    let body = wait_body(&client, &url, &server.log_path);
    assert!(
        body.contains("hello server"),
        "expected SSR page text:\n{body}"
    );

    let debug_port = free_port();
    let chrome_dir = tempfile::tempdir().expect("chrome profile dir");
    let _chrome = spawn_chrome(debug_port, chrome_dir.path());
    let mut cdp = Cdp::connect(debug_port);
    cdp.navigate(&url);
    cdp.wait_eval_eq(
        "document.querySelector('#server-title') && document.querySelector('#server-title').textContent",
        "hello server",
        Duration::from_secs(45),
    );
    cdp.wait_eval_eq(
        "document.querySelector('#counter') ? 'yes' : 'no'",
        "yes",
        Duration::from_secs(30),
    );

    let click_deadline = Instant::now() + Duration::from_secs(30);
    let mut count = String::new();
    while Instant::now() < click_deadline {
        let _ = cdp.eval("document.getElementById('counter') && document.getElementById('counter').click()");
        count = cdp.eval_string(
            "document.getElementById('counter') && document.getElementById('counter').textContent",
        );
        if count == "1" || count == "2" || count == "3" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    while count != "3" && Instant::now() < click_deadline {
        let _ = cdp.eval("document.getElementById('counter') && document.getElementById('counter').click()");
        count = cdp.eval_string(
            "document.getElementById('counter') && document.getElementById('counter').textContent",
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(count, "3", "pre-edit clicks must stick in island React state");

    let filled = cdp.eval_string(
        r#"(function(){
            var el = document.getElementById('outside-input');
            el.focus();
            el.value = 'kept';
            return el.value;
        })()"#,
    );
    assert_eq!(filled, "kept");
    let _ = cdp.eval("window.scrollTo(0, 480); 'ok'");
    let scroll_before = cdp.eval_string("String(window.scrollY || window.pageYOffset || 0)");

    cdp.navigations = 0;
    cdp.drain();

    fs::write(
        root.path().join("app/page.dsx"),
        r#"export fn Page() ReactNode {
  return <section>
      <h1 id="server-title">hello refreshed</h1>
      <form>
        <input id="outside-input" name="note" value="initial" />
      </form>
      <div id="spacer" class="spacer"></div>
    </section>
}
"#,
    )
    .unwrap();

    cdp.wait_eval_eq(
        "document.querySelector('#server-title') && document.querySelector('#server-title').textContent",
        "hello refreshed",
        Duration::from_secs(45),
    );
    assert_eq!(
        cdp.eval_string(
            "document.getElementById('counter') && document.getElementById('counter').textContent"
        ),
        "3",
        "morph must not recreate the hydrated counter island"
    );
    assert_eq!(
        cdp.eval_string(
            "document.getElementById('outside-input') && document.getElementById('outside-input').value"
        ),
        "kept",
        "form input outside the island must keep its value"
    );
    let scroll_after = cdp.eval_string("String(window.scrollY || window.pageYOffset || 0)");
    let before_y: i64 = scroll_before.parse().unwrap_or(0);
    let after_y: i64 = scroll_after.parse().unwrap_or(0);
    assert!(
        (before_y - after_y).abs() < 80,
        "scroll should survive morph: before={scroll_before} after={scroll_after}"
    );
    cdp.drain();
    assert_eq!(
        cdp.navigations, 0,
        "a server-text edit must morph without frame navigation"
    );

    fs::write(
        root.path().join("src/ui/Counter.dsx"),
        r#"export fn Counter() ReactNode {
  const pair = useState(10)
  const n = pair[0]
  const setN = pair[1]
  return <button type="button" id="counter" onClick={fn() void {
      setN(n + 1)
    }}>{n}</button>
}
"#,
    )
    .unwrap();

    let nav_deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < nav_deadline && cdp.navigations == 0 {
        cdp.drain();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        cdp.navigations >= 1,
        "editing the island module must full-reload (frame navigation); log:\n{}",
        fs::read_to_string(&server.log_path).unwrap_or_default()
    );
}
