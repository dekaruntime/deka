#![cfg(feature = "dev-server")]

//! deka#936: Fast Refresh over the real `deka dev` HTTP + HMR surface.

use reqwest::blocking::Client;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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

fn worktree_dir(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fast-refresh-it")
        .join(format!("{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create worktree-local test dir");
    root
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read ephemeral port")
        .port()
}

fn write_project(root: &Path) {
    fs::write(
        root.join("deka.json"),
        r#"{"name":"fast-refresh","security":{"prompt":false}}"#,
    )
    .unwrap();
    fs::write(
        root.join("index.js"),
        r#"globalThis.app = {
  async fetch() {
    const html = `<!doctype html>
<html>
  <head><meta charset="utf-8"><title>fast-refresh</title></head>
  <body>
    <div id="root"></div>
    <script type="module">
      import { createElement as h } from "react";
      import { createRoot } from "react-dom/client";
      import { App } from "/_deka/hmr/module/App.js";
      createRoot(document.getElementById("root")).render(h(App));
    </script>
  </body>
</html>`;
    return new Response(html, { headers: { "content-type": "text/html; charset=utf-8" } });
  },
};
"#,
    )
    .unwrap();
    fs::write(
        root.join("App.js"),
        r#"import { createElement as h } from "react";
import { Counter } from "./Counter.js";
import { Label } from "./Label.js";
export function App() {
  return h("div", { id: "app-root" }, h(Counter), h(Label));
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("Counter.js"),
        r#"import { useState, createElement as h } from "react";
export function Counter() {
  const [n, setN] = useState(0);
  return h("button", { id: "counter", onClick: () => setN(n + 1) }, String(n));
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("Label.js"),
        r#"import { createElement as h } from "react";
export function Label() {
  return h("p", { id: "label" }, "hello");
}
"#,
    )
    .unwrap();
}

fn spawn_dev(root: &Path, port: u16) -> ServeProcess {
    // Log writes must not feed back into the recursive source watcher.
    fs::create_dir_all(root.join(".cache")).expect("create dev log directory");
    let log_path = root.join(".cache/dev.log");
    let log = fs::File::create(&log_path).expect("create dev log");
    let mut command = Command::new(cli_bin());
    command
        .args(["dev", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root)
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log));
    let dsc_beside_cli = Path::new(cli_bin()).with_file_name("dsc");
    if dsc_beside_cli.is_file() {
        command.env("DEKA_DSC", dsc_beside_cli);
    }
    ServeProcess(command.spawn().expect("spawn deka dev"))
}

fn wait_ok(client: &Client, url: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = String::new();
    while Instant::now() < deadline {
        match client.get(url).send() {
            Ok(response) => {
                let status = response.status();
                let body = response.text().unwrap_or_default();
                if status.is_success() {
                    return body;
                }
                last = format!("status {status} body={body}");
            }
            Err(err) => last = err.to_string(),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {url}: {last}");
}

#[test]
fn deka_dev_serves_react_vendor_and_refresh_wrapped_modules() {
    let root = worktree_dir("serve-modules");
    write_project(&root);
    let port = free_port();
    let _server = spawn_dev(&root, port);
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let page = wait_ok(&client, &format!("http://127.0.0.1:{port}/"));
    assert!(
        page.contains("__deka_refresh_preamble"),
        "HTML must inject the Fast Refresh preamble:\n{page}"
    );
    assert!(
        page.contains("/_deka/react/react.js"),
        "HTML must expose the runtime-shipped React import map:\n{page}"
    );

    let react = wait_ok(
        &client,
        &format!("http://127.0.0.1:{port}/_deka/react/react.js"),
    );
    assert!(react.contains("useState"), "vendored react: {react:.200}");
    assert!(react.contains("19.1.1"));

    let refresh = wait_ok(
        &client,
        &format!("http://127.0.0.1:{port}/_deka/react/refresh-runtime.js"),
    );
    assert!(refresh.contains("performReactRefresh"));
    assert!(refresh.contains("injectIntoGlobalHook"));

    let jsx_dev = wait_ok(
        &client,
        &format!("http://127.0.0.1:{port}/_deka/react/jsx-dev-runtime.js"),
    );
    assert!(jsx_dev.contains("jsxDEV"));

    let counter = wait_ok(
        &client,
        &format!("http://127.0.0.1:{port}/_deka/hmr/module/Counter.js"),
    );
    assert!(
        counter.contains("$RefreshReg$(Counter, \"Counter\")"),
        "dev module must register the component:\n{counter}"
    );
    assert!(counter.contains("__dekaRefreshBoundary = true"));
    assert!(counter.contains("useState"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn editing_a_component_recompiles_the_served_module() {
    let root = worktree_dir("edit-module");
    write_project(&root);
    let port = free_port();
    let _server = spawn_dev(&root, port);
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let before = wait_ok(
        &client,
        &format!("http://127.0.0.1:{port}/_deka/hmr/module/Label.js"),
    );
    assert!(before.contains("hello"), "{before}");

    fs::write(
        root.join("Label.js"),
        r#"import { createElement as h } from "react";
export function Label() {
  return h("p", { id: "label" }, "hello world");
}
"#,
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = before.clone();
    while Instant::now() < deadline {
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/_deka/hmr/module/Label.js"))
            .send()
        {
            if let Ok(body) = response.text() {
                last = body;
                if last.contains("hello world") && last.contains("$RefreshReg$(Label, \"Label\")") {
                    let _ = fs::remove_dir_all(&root);
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("edited Label.js was not recompiled for Fast Refresh; last body:\n{last}");
}

#[path = "support/app_router.rs"]
mod app_router;

// Keep watcher fixtures outside target/.cache: those path segments are
// intentionally ignored by the production watcher.
#[test]
fn app_router_headers_and_document_edit_reload_are_visible_over_http_and_ws() {
    let root = tempfile::Builder::new()
        .prefix("deka-refresh-955-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .unwrap();
    app_router::init_project(root.path());
    let page_path = root.path().join("app/page.dsx");
    fs::write(
        &page_path,
        "export fn Page() { return <p>before-edit-955</p> }\n",
    )
    .unwrap();
    let port = free_port();
    let _server = spawn_dev(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{port}/");
    let first = wait_ok(&client, &url);
    assert!(first.contains("before-edit-955"), "{first}");
    // Warm the server isolate before the edit; a request immediately after
    // notification must not reuse that isolate's old module graph.
    let response = client.get(&url).send().unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let html = response.text().unwrap();
    assert!(html.contains("__deka_refresh_preamble"), "{html}");
    assert!(html.contains("/_deka/hmr"), "{html}");

    let response = client
        .get(&url)
        .header("Accept", "text/x-deka-fragment")
        .send()
        .unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    assert!(response.status().is_success());
    let fragment: serde_json::Value = response.json().unwrap();
    assert!(
        fragment["html"]
            .as_str()
            .unwrap()
            .contains("before-edit-955")
    );
    assert!(
        !fragment["html"]
            .as_str()
            .unwrap()
            .contains("__deka_refresh_preamble")
    );

    let (mut socket, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/_deka/hmr")).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream
            .set_read_timeout(Some(Duration::from_secs(45)))
            .unwrap();
    }
    fs::write(
        &page_path,
        "export fn Page() { return <p>after-edit-955</p> }\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        assert!(Instant::now() < deadline, "no document-edit notification");
        let message = socket.read().expect("receive HMR frame");
        if !message.is_text() {
            continue;
        }
        let payload: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        if !payload["paths"].as_array().is_some_and(|paths| {
            paths.iter().any(|path| {
                path.as_str()
                    .is_some_and(|path| path.ends_with("/app/page.dsx"))
            })
        }) {
            continue;
        }
        // Post-#956, a non-island document edit is pushed as an html-update
        // (state-preserving morph), not a js-update reload cycle.
        assert_eq!(payload["type"], "html-update", "{payload}");
        assert!(
            payload["html"]
                .as_str()
                .is_some_and(|html| html.contains("after-edit-955")),
            "{payload}"
        );
        // No retry/poll after the WS frame: stale content here is a regression.
        let response = client.get(&url).send().unwrap();
        assert!(response.status().is_success());
        let html = response.text().unwrap();
        assert!(
            html.contains("after-edit-955") && !html.contains("before-edit-955"),
            "first post-notification response is stale: {html}"
        );
        // The shipped refresh client's js-update contract is covered by the
        // react_refresh lib tests; the html-update morph path has its own
        // real-topology e2e in server_fast_refresh.rs.
        break;
    }
}

#[test]
fn handler_reload_notification_observes_evicted_server_isolates() {
    let root = tempfile::Builder::new()
        .prefix("deka-reload-955-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .unwrap();
    fs::write(
        root.path().join("deka.json"),
        r#"{"name":"reload-955","security":{"prompt":false}}"#,
    )
    .unwrap();
    let handler = root.path().join("index.js");
    let source = |marker: &str| {
        format!("globalThis.app = {{ fetch() {{ return new Response({marker:?}); }} }};\n")
    };
    fs::write(&handler, source("before-reload-955")).unwrap();
    let port = free_port();
    let _server = spawn_dev(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{port}/");
    assert_eq!(wait_ok(&client, &url), "before-reload-955");
    let (mut socket, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/_deka/hmr")).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream
            .set_read_timeout(Some(Duration::from_secs(45)))
            .unwrap();
    }
    fs::write(&handler, source("after-reload-955")).unwrap();
    // No React boundary or patchable HTML container: exercise the watcher's
    // push_js_update(false) -> notify_hmr_changed -> reload WS path.
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        assert!(Instant::now() < deadline, "no handler reload notification");
        let frame = socket.read().expect("receive reload frame");
        if !frame.is_text() {
            continue;
        }
        let payload: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
        if !payload["paths"].as_array().is_some_and(|paths| {
            paths.iter().any(|path| {
                path.as_str()
                    .is_some_and(|path| path.ends_with("/index.js"))
            })
        }) {
            continue;
        }
        assert_eq!(payload["type"], "reload", "{payload}");
        let response = client.get(&url).send().unwrap();
        assert!(response.status().is_success());
        assert_eq!(
            response.text().unwrap(),
            "after-reload-955",
            "first request after reload notification must see saved source"
        );
        break;
    }
}

/// deka#1067: one editor save must run one watch/evict/hmr cycle, not one
/// per raw filesystem event. Atomic-save editors (and `sed -i`) write a temp
/// file next to the target and rename it over — on Linux/inotify that alone
/// delivers `Modify(Data)`, `Access(Close(Write))`, `Modify(Name(From))`,
/// `Modify(Name(To))` and a synthesized `Modify(Name(Both))`, five raw events
/// for one logical change. This exercises that exact save shape over the
/// real `deka dev` process (not an in-process unit call) and asserts exactly
/// one HMR notification reaches the browser, and exactly one `[hmr] changed`
/// line — with the `[watch]` bookkeeping lines suppressed by default — reaches
/// the terminal.
#[test]
fn atomic_rename_save_produces_exactly_one_hmr_cycle() {
    let root = tempfile::Builder::new()
        .prefix("deka-single-cycle-1067-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .unwrap();
    app_router::init_project(root.path());
    let layout_path = root.path().join("app/layout.dsx");
    let port = free_port();
    let _server = spawn_dev(root.path(), port);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{port}/");
    wait_ok(&client, &url);

    let (mut socket, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/_deka/hmr")).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
    }

    // Simulate an atomic-save editor: write the new content to a sibling
    // temp file, then rename it over the real target. `fs::write` alone
    // (open+write+close on the same path) would not reproduce the bug.
    let tmp_path = root.path().join("app/.layout.dsx.tmp-1067");
    fs::write(
        &tmp_path,
        "interface LayoutProps {\n  children: ReactNode;\n}\nexport fn Layout(props: LayoutProps) {\n  return <main class=\"edited-1067\">{props.children}</main>\n}\n",
    )
    .unwrap();
    fs::rename(&tmp_path, &layout_path).unwrap();

    // Collect every HMR frame that mentions layout.dsx over a window long
    // enough to observe a regression (each duplicate cycle would arrive
    // within milliseconds of the first) but short enough not to wait out a
    // legitimately quiet socket.
    let collect_deadline = Instant::now() + Duration::from_secs(3);
    let mut layout_notifications = Vec::new();
    while Instant::now() < collect_deadline {
        match socket.read() {
            Ok(message) if message.is_text() => {
                let payload: serde_json::Value =
                    serde_json::from_str(message.to_text().unwrap()).unwrap();
                let mentions_layout = payload["paths"].as_array().is_some_and(|paths| {
                    paths.iter().any(|path| {
                        path.as_str()
                            .is_some_and(|path| path.ends_with("/app/layout.dsx"))
                    })
                });
                if mentions_layout {
                    layout_notifications.push(payload);
                }
            }
            Ok(_) => continue,
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(err) => panic!("HMR socket error while collecting notifications: {err}"),
        }
    }

    assert_eq!(
        layout_notifications.len(),
        1,
        "one atomic-rename save must produce exactly one HMR notification, got {}: {:?}",
        layout_notifications.len(),
        layout_notifications
    );

    // The terminal-facing side of the same bug: exactly one `[hmr] changed`
    // line for the save, and the `[watch]` bookkeeping lines suppressed
    // because `--debug` was not passed (deka#1067 part B).
    let log = fs::read_to_string(root.path().join(".cache/dev.log")).unwrap_or_default();
    let hmr_lines: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("[hmr] changed") && line.contains("layout.dsx"))
        .collect();
    assert_eq!(
        hmr_lines.len(),
        1,
        "expected exactly one [hmr] changed line for the save, got: {hmr_lines:?}\nfull log:\n{log}"
    );
    assert!(
        !log.contains("[watch] evicted"),
        "bookkeeping must stay behind --debug, not print by default:\n{log}"
    );
}
