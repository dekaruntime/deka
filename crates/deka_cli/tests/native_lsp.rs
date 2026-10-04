//! Real stdio protocol tests: the shipped CLI must validate native lowering,
//! open buffers and dependency buffers without evaluating a program.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::Duration,
};
use tempfile::TempDir;
struct Server {
    child: Child,
    input: Option<ChildStdin>,
    messages: Receiver<Value>,
    reader: Option<JoinHandle<std::io::Result<()>>>,
    next: u64,
}
impl Server {
    fn new(cwd: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_deka"))
            .args(["lsp", "--stdio"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start lsp");
        let input = child.stdin.take().expect("stdin");
        let output = child.stdout.take().expect("stdout");
        let (tx, messages) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut length = None;
                loop {
                    let mut line = String::new();
                    if output.read_line(&mut line)? == 0 {
                        return Ok(());
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Content-Length: ") {
                        length = value.trim().parse::<usize>().ok();
                    }
                }
                let length =
                    length.ok_or_else(|| std::io::Error::other("stdout is not LSP framing"))?;
                let mut bytes = vec![0; length];
                output.read_exact(&mut bytes)?;
                let value: Value = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
                if tx.send(value).is_err() {
                    return Ok(());
                }
            }
        });
        let mut server = Self {
            child,
            input: Some(input),
            messages,
            reader: Some(reader),
            next: 1,
        };
        let initialized =
            server.request("initialize", json!({"capabilities":{},"rootUri":uri(cwd)}));
        assert_eq!(initialized["result"]["capabilities"]["textDocumentSync"], 1);
        server.notify("initialized", json!({}));
        server
    }
    fn send(&mut self, value: Value) {
        let bytes = serde_json::to_vec(&value).expect("json");
        let input = self.input.as_mut().expect("open input");
        write!(input, "Content-Length: {}\r\n\r\n", bytes.len()).expect("header");
        input.write_all(&bytes).expect("body");
        input.flush().expect("flush");
    }
    fn notify(&mut self, method: &str, params: Value) {
        let mut message = json!({"jsonrpc":"2.0","method":method});
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(message);
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let mut message = json!({"jsonrpc":"2.0","id":id,"method":method});
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(message);
        loop {
            let value = self
                .messages
                .recv_timeout(Duration::from_secs(30))
                .expect("LSP response");
            if value["id"] == id {
                assert!(value.get("error").is_none(), "{value}");
                return value;
            }
        }
    }
    fn open(&mut self, path: &std::path::Path, source: &str) {
        self.notify("textDocument/didOpen", json!({"textDocument":{"uri":uri(path),"languageId":"dekascript","version":1,"text":source}}));
    }
    fn change(&mut self, path: &std::path::Path, source: &str) {
        self.notify("textDocument/didChange", json!({"textDocument":{"uri":uri(path),"version":2},"contentChanges":[{"text":source}]}));
    }
    fn diagnostic(&mut self, path: &std::path::Path) -> Value {
        self.request(
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":uri(path)}}),
        )["result"]
            .clone()
    }
    fn publish(&mut self, path: &std::path::Path) -> Value {
        loop {
            let value = self
                .messages
                .recv_timeout(Duration::from_secs(30))
                .expect("diagnostic notification");
            if value["method"] == "textDocument/publishDiagnostics"
                && value["params"]["uri"] == uri(path)
            {
                return value["params"]["diagnostics"].clone();
            }
        }
    }
    fn finish(&mut self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        // End input after the exit notification, as editors do on transport close.
        drop(self.input.take());
        assert!(self.child.wait().expect("wait lsp").success());
        self.reader
            .take()
            .expect("reader")
            .join()
            .expect("reader join")
            .expect("valid framing");
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
fn uri(path: &std::path::Path) -> String {
    let value = reqwest::Url::from_file_path(path).expect("file URI");
    // A real client/parser normalizes dot segments in a file URI.
    reqwest::Url::parse(value.as_str())
        .expect("normalized URI")
        .to_string()
}

fn setup() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("scratch");
    let path = dir.path().join("main.ds");
    std::fs::write(&path, "echo(\"disk\");").expect("disk source");
    (dir, path)
}
#[test]
fn native_lowering_refusal_matches_check_and_unsaved_fix_clears_it() {
    let (dir, path) = setup();
    let source = "\n\nconst value = parseNumber(\"42\");";
    std::fs::write(&path, source).expect("source");
    let check = Command::new(env!("CARGO_BIN_EXE_deka"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("check");
    assert!(!check.status.success());
    let mut lsp = Server::new(dir.path());
    lsp.open(&path, source);
    let diagnostics = lsp.publish(&path);
    assert_eq!(
        diagnostics[0]["range"]["start"],
        json!({"line":2,"character":14})
    );
    assert!(
        diagnostics[0]["message"]
            .as_str()
            .expect("message")
            .contains("unknown built-in parseNumber")
    );
    assert!(
        String::from_utf8(check.stderr)
            .expect("stderr")
            .contains(diagnostics[0]["message"].as_str().expect("message"))
    );
    lsp.change(
        &path,
        "import {echo} from \"io\"; echo(\"must not execute\");",
    );
    assert!(lsp.publish(&path).as_array().expect("array").is_empty());
    assert!(
        lsp.diagnostic(&path)["items"]
            .as_array()
            .expect("items")
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("disk unchanged"),
        source
    );
    lsp.finish();
}
#[test]
fn unsaved_dependency_is_checked_at_its_own_uri_and_cleared() {
    let (dir, path) = setup();
    let helper = dir.path().join("helper.ds");
    std::fs::write(&helper, "export fn value() number { return 7; }").expect("helper");
    let entry = "import { value } from \"./helper.ds\"; const result = value();";
    std::fs::write(&path, entry).expect("entry");
    let mut lsp = Server::new(dir.path());
    lsp.open(&path, entry);
    lsp.publish(&path);
    let bad = "const unsupported=parseNumber(\"7\"); export fn value() number { return 7; }";
    lsp.open(&helper, bad);
    let errors = lsp.publish(&helper);
    assert!(
        errors[0]["message"]
            .as_str()
            .expect("message")
            .contains("unknown built-in parseNumber")
    );
    let pull = lsp.diagnostic(&path);
    assert!(
        pull["relatedDocuments"][uri(&helper)]["items"][0]["message"]
            .as_str()
            .expect("dependency error")
            .contains("parseNumber")
    );
    lsp.change(&helper, "export fn value() number { return 8; }");
    assert!(lsp.publish(&helper).as_array().expect("clear").is_empty());
    assert!(
        lsp.diagnostic(&path)["items"]
            .as_array()
            .expect("entry")
            .is_empty()
    );
    lsp.finish();
}
#[test]
fn new_unsaved_file_and_unicode_columns_work_without_shadow_files() {
    let (dir, _) = setup();
    let path = dir.path().join("new.ds");
    let source = "const greeting = \"😀\"; const value = parseNumber(\"7\");";
    let mut lsp = Server::new(dir.path());
    lsp.open(&path, source);
    let errors = lsp.publish(&path);
    let expected: usize = source[..source.find("parseNumber").expect("call")]
        .encode_utf16()
        .count();
    assert_eq!(errors[0]["range"]["start"]["character"], expected);
    assert!(!path.exists());
    lsp.finish();
}
#[test]
fn navigation_and_completion_use_unsaved_sources_and_native_catalog() {
    let (dir, path) = setup();
    let helper = dir.path().join("helper.ds");
    std::fs::write(&helper, "export fn original(): string { return \"disk\"; }").expect("helper");
    let entry = "import { greeting } from \"./helper.ds\";\nconst text = greeting();";
    let mut lsp = Server::new(dir.path());
    lsp.open(
        &helper,
        "export fn greeting(): string { return \"buffer\"; }",
    );
    lsp.publish(&helper);
    lsp.open(&path, entry);
    lsp.publish(&path);
    let position = json!({"textDocument":{"uri":uri(&path)},"position":{"line":1,"character":15}});
    let hover = lsp.request("textDocument/hover", position.clone());
    assert!(
        hover["result"]["contents"]["value"]
            .as_str()
            .expect("hover")
            .contains("greeting")
    );
    let definition = lsp.request("textDocument/definition", position);
    assert_eq!(definition["result"]["uri"], uri(&helper));
    lsp.change(&path, "import { sha } from \"crypto\";");
    let items = lsp.request(
        "textDocument/completion",
        json!({"textDocument":{"uri":uri(&path)},"position":{"line":0,"character":12}}),
    );
    let labels: Vec<_> = items["result"]
        .as_array()
        .expect("completion")
        .iter()
        .map(|item| item["label"].as_str().expect("label"))
        .collect();
    assert!(labels.contains(&"sha256"));
    assert!(!labels.contains(&"readFileSync"));
    assert!(!labels.contains(&"useState"));
    lsp.finish();
}

#[test]
fn changing_one_open_document_keeps_another_documents_errors() {
    let (dir, _) = setup();
    let first = dir.path().join("a.ds");
    let second = dir.path().join("z.ds");
    let mut lsp = Server::new(dir.path());
    lsp.open(&first, "const a=parseNumber(\"1\");");
    assert!(
        !lsp.publish(&first)
            .as_array()
            .expect("first errors")
            .is_empty()
    );
    lsp.open(&second, "const z=parseNumber(\"2\");");
    assert!(
        !lsp.publish(&first)
            .as_array()
            .expect("first still invalid")
            .is_empty()
    );
    assert!(
        !lsp.publish(&second)
            .as_array()
            .expect("second errors")
            .is_empty()
    );
    lsp.change(&second, "const z=2;");
    assert!(
        !lsp.publish(&first)
            .as_array()
            .expect("retain first errors")
            .is_empty()
    );
    assert!(
        lsp.publish(&second)
            .as_array()
            .expect("clear second errors")
            .is_empty()
    );
    lsp.finish();
}
