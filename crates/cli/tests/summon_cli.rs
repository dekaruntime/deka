//! All summon sources are local; CI never contacts a CDN.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("deka.json"),
        "{\"name\":\"consumer\",\"custom\":42,\"dependencies\":{}}",
    )
    .unwrap();
    fs::write(
        dir.path().join("deka.lock"),
        "{\"lockfileVersion\":1,\"packages\":{}}",
    )
    .unwrap();
    dir
}
fn run(dir: &Path, args: &[&str], input: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cli"))
        .args(args)
        .current_dir(dir)
        .env("DEKA_SECURITY_NO_PROMPT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
fn read(dir: &Path, file: &str) -> Value {
    serde_json::from_slice(&fs::read(dir.join(file)).unwrap()).unwrap()
}
fn source(dir: &Path, name: &str, code: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, code).unwrap();
    reqwest::Url::from_file_path(path).unwrap().to_string()
}
fn archive(dir: &Path, files: &[(&str, &str)]) -> String {
    let path = dir.join("download.tgz");
    let gzip = flate2::write::GzEncoder::new(
        fs::File::create(&path).unwrap(),
        flate2::Compression::default(),
    );
    let mut tar = tar::Builder::new(gzip);
    for (name, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, name, content.as_bytes())
            .unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();
    reqwest::Url::from_file_path(path).unwrap().to_string()
}

#[test]
fn http_downloads_once_and_lock_manifest_round_trip_offline() {
    let dir = project();
    let code = b"export function answer() { return 42; }\n";
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/answer.mjs", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = done.clone();
    let server = thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            if let Ok((mut stream, _)) = listener.accept() {
                calls.fetch_add(1, Ordering::SeqCst);
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    code.len()
                )
                .unwrap();
                stream.write_all(code).unwrap();
            } else {
                thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    });
    let output = run(dir.path(), &["summon", &format!("url:{url}")], None);
    done.store(true, Ordering::SeqCst);
    server.join().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(dir.path().join("js_modules/answer/index.mjs")).unwrap(),
        code
    );
    let manifest = read(dir.path(), "deka.json");
    let lock = read(dir.path(), "deka.lock");
    assert_eq!(manifest["custom"], 42);
    assert_eq!(manifest["dependencies"]["@js/answer"], format!("url:{url}"));
    assert_eq!(
        lock["packages"]["@js/answer"][3],
        format!("sha256-{}", STANDARD.encode(Sha256::digest(code)))
    );
    let parsed = pm::lock::read_lockfile_at(&dir.path().join("deka.lock"));
    pm::lock::write_lockfile_at(&dir.path().join("deka.lock"), &parsed).unwrap();
    assert_eq!(read(dir.path(), "deka.lock"), lock);
    let installed = run(dir.path(), &["install", "--locked"], None);
    assert!(installed.status.success(), "{}", text(&installed));
    assert_eq!(read(dir.path(), "deka.lock"), lock);
    fs::write(
        dir.path().join("js_modules/answer/index.mjs"),
        "export const answer = 0;",
    )
    .unwrap();
    let tampered = run(dir.path(), &["install", "--locked"], None);
    assert!(!tampered.status.success());
    assert!(
        text(&tampered).contains("integrity mismatch"),
        "{}",
        text(&tampered)
    );
    assert!(!dir.path().join("js_modules/answer/summon.d.ds").exists());
}

#[test]
fn conflict_errors_or_prompts_for_a_new_name_without_overwrite() {
    let dir = project();
    let fixtures = tempfile::tempdir().unwrap();
    let url = source(
        fixtures.path(),
        "answer.mjs",
        "export function answer() { return 42; }",
    );
    assert!(run(dir.path(), &["summon", &url], None).status.success());
    let before = fs::read(dir.path().join("deka.lock")).unwrap();
    let refused = run(dir.path(), &["summon", &url], None);
    assert!(!refused.status.success());
    assert!(text(&refused).contains("conflict"));
    assert_eq!(fs::read(dir.path().join("deka.lock")).unwrap(), before);
    let cancelled = run(dir.path(), &["summon", &url, "--prompt"], Some("\n"));
    assert!(!cancelled.status.success());
    assert_eq!(fs::read(dir.path().join("deka.lock")).unwrap(), before);
    let renamed = run(
        dir.path(),
        &["summon", &url, "--prompt"],
        Some("answer-two\n"),
    );
    assert!(renamed.status.success(), "{}", text(&renamed));
    assert!(text(&renamed).contains("New vendor name"));
    assert!(dir.path().join("js_modules/answer/index.mjs").is_file());
    assert!(dir.path().join("js_modules/answer-two/index.mjs").is_file());
}

#[test]
fn transitive_refusal_lists_static_reexports_dynamic_and_require_without_mutation() {
    let dir = project();
    let fixtures = tempfile::tempdir().unwrap();
    let url = source(
        fixtures.path(),
        "outer.mjs",
        "import './a.mjs'; export * from './b.mjs'; export {x} from 'bare'; import('https://example.invalid/inner.mjs'); require('./c.js'); import(variable);",
    );
    let before = fs::read(dir.path().join("deka.lock")).unwrap();
    let output = run(dir.path(), &["summon", &url], None);
    assert!(!output.status.success());
    for expected in [
        "no transitive fetching",
        "./a.mjs",
        "./b.mjs",
        "bare",
        "https://example.invalid/inner.mjs",
        "./c.js",
        "non-literal",
    ] {
        assert!(text(&output).contains(expected), "{}", text(&output));
    }
    assert_eq!(fs::read(dir.path().join("deka.lock")).unwrap(), before);
    assert!(!dir.path().join("js_modules").exists());
    assert_eq!(read(dir.path(), "deka.json")["dependencies"], json!({}));
}

#[test]
fn package_name_module_precedence_relative_graph_and_runtime_resolution() {
    let dir = project();
    let fixtures = tempfile::tempdir().unwrap();
    let url = archive(
        fixtures.path(),
        &[
            (
                "package/package.json",
                r#"{"name":"chosen","module":"src/module.mjs","main":"missing.js"}"#,
            ),
            (
                "package/src/module.mjs",
                "export { answer } from './inner.mjs';",
            ),
            (
                "package/src/inner.mjs",
                "export function answer() { return 42; }",
            ),
        ],
    );
    let output = run(dir.path(), &["summon", &url], None);
    assert!(output.status.success(), "{}", text(&output));
    let resolved =
        deka_modules::module_spec::resolve_summoned_js_module_file(dir.path(), "@js/chosen")
            .unwrap();
    assert!(resolved.ends_with("js_modules/chosen/src/module.mjs"));
    fs::write(
        dir.path().join("main.js"),
        "import { answer } from '@js/chosen'; console.log(answer());",
    )
    .unwrap();
    let output = run(dir.path(), &["run", "main.js"], None);
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("42"), "{}", text(&output));
}

#[test]
fn explicitly_summoned_import_is_accepted_but_unlocked_one_is_refused() {
    let dir = project();
    let fixtures = tempfile::tempdir().unwrap();
    let inner = source(
        fixtures.path(),
        "inner.mjs",
        "export function answer() { return 42; }",
    );
    let outer = source(
        fixtures.path(),
        "outer.mjs",
        "export { answer } from '@js/inner';",
    );
    assert!(!run(dir.path(), &["summon", &outer], None).status.success());
    assert!(run(dir.path(), &["summon", &inner], None).status.success());
    let output = run(dir.path(), &["summon", &outer], None);
    assert!(output.status.success(), "{}", text(&output));
}

#[test]
fn malformed_lock_invalid_js_and_deferred_stages_fail_without_changes() {
    let dir = project();
    let fixtures = tempfile::tempdir().unwrap();
    let url = source(fixtures.path(), "bad.mjs", "export function {");
    for input in [&url[..], "jsr:@example/library", "infer"] {
        let output = run(dir.path(), &["summon", input], None);
        assert!(!output.status.success(), "{}", text(&output));
    }
    fs::write(dir.path().join("deka.lock"), "broken").unwrap();
    let good = source(fixtures.path(), "good.mjs", "export const value = 1;");
    assert!(!run(dir.path(), &["summon", &good], None).status.success());
    assert_eq!(
        fs::read_to_string(dir.path().join("deka.lock")).unwrap(),
        "broken"
    );
    assert!(!dir.path().join("js_modules").exists());
}

#[test]
fn archive_cannot_supply_pms_transaction_marker_as_its_entry() {
    let dir = project();
    let fixtures = tempfile::tempdir().unwrap();
    let url = archive(
        fixtures.path(),
        &[
            (
                "package/package.json",
                r#"{"name":"marker","main":".deka-staged-package"}"#,
            ),
            (
                "package/.deka-staged-package",
                "export function answer() { return 42; }",
            ),
        ],
    );
    let output = run(dir.path(), &["summon", &url], None);
    assert!(!output.status.success());
    assert!(
        text(&output).contains("reserved pm transaction marker"),
        "{}",
        text(&output)
    );
    assert_eq!(read(dir.path(), "deka.json")["dependencies"], json!({}));
    assert!(!dir.path().join("js_modules").exists());
}
