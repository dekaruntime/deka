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
    let log_path = root.join("dev.log");
    let log = fs::File::create(&log_path).expect("create dev log");
    let mut command = Command::new(cli_bin());
    command
        .args(["dev", ".", "--port", &port.to_string(), "--no-prompt"])
        .current_dir(root)
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
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
