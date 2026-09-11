//! deka#750: the browser-fetched client payload for an island page.
//!
//! Mechanism under test (crates/runtime/src/islands.rs):
//! - PAUSED (deka#881 DECIDE-1): the framework — islands included — is
//!   paused, so dist currently ships the same readable, unpruned `ui/*`
//!   chunks as dev (the deka#750 export-prune + minify pipeline was deleted
//!   with crates/bundler and must be restored through dsc's optimizer stage,
//!   not reinstated locally). The deka#750 byte budgets and the
//!   SSR-producer-absence assertions are parked with it.
//! - `ui/island-marker.js` is NOT split: its SSR producers
//!   (`formatIslandStart`, `formatIslandEnd`, `encodeB64`, `utf8Bytes`) are
//!   plain function declarations, so a future pruner can drop them once
//!   browser chunks keep only `parseIslandMarker`.
//! - `ClientAssetFlavor::Dev` (`deka serve`) writes the readable sources
//!   byte-identical, and dist currently matches it (see above).

use std::collections::BTreeSet;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use tempfile::TempDir;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// SSR-side producers of `ui/island-marker.js`. While dist pruning is paused
/// (deka#881 DECIDE-1) they appear in every flavor's chunks; the deka#750
/// assertion that they are absent from dist returns with the optimize stage.
const PRODUCER_SYMBOLS: [&str; 4] = [
    "formatIslandStart",
    "formatIslandEnd",
    "encodeB64",
    "utf8Bytes",
];

/// The counter island: the only `ui/reactive` usage pattern that both passes
/// the dsc 0.7 checker and hydrates in the browser (module-level signal,
/// `{get()}` so the compiler wraps reads in `live()`).
const COUNTER_PAGE: &str = r#"import { signal } from "ui/reactive"
const pair = signal(1)
const get = pair[0]
const set = pair[1]

export fn Counter() {
    return <button onClick={fn() { set(get() + 1) }}>{get()}</button>
}

export fn Page() {
    return <main><Counter client:load /></main>
}
"#;

fn init_project(dir: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(dir)
        .output()
        .expect("run deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `deka init` scaffold plus the counter island page.
fn write_counter_project(dir: &Path) {
    init_project(dir);
    fs::write(dir.join("app").join("page.dsx"), COUNTER_PAGE).expect("write counter page");
}

fn run_build(dir: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(dir)
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// `<dir>/<stem>.<10-hex>.<ext>`, the content-hashed asset naming scheme.
fn find_hashed_asset(dir: &Path, stem: &str, ext: &str) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(without_ext) = name.strip_suffix(ext) else {
            return None;
        };
        let Some((file_stem, hash)) = without_ext.strip_suffix('.').unwrap_or("").rsplit_once('.')
        else {
            return None;
        };
        let is_hash = hash.len() == 10
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        (file_stem == stem && is_hash).then_some(entry.path())
    })
}

/// Rewrite hashed sibling references (`./jsx.91a1195be0.js`) back to their
/// logical form (`./jsx.js`) so a written chunk can be compared against the
/// raw deka_ui source it was hashed from.
fn strip_sibling_hashes(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for (index, seg) in body.split('"').enumerate() {
        if index > 0 {
            out.push('"');
        }
        if index % 2 == 0 {
            out.push_str(seg);
            continue;
        }
        let mut replaced = false;
        if let Some(rel) = seg.strip_prefix("./") {
            if let Some((stem, hash_ext)) = rel.rsplit_once('.') {
                if let Some((file_stem, hash)) = stem.rsplit_once('.') {
                    let is_hash = hash.len() == 10
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                    if is_hash && hash_ext == "js" {
                        out.push_str("./");
                        out.push_str(file_stem);
                        out.push_str(".js");
                        replaced = true;
                    }
                }
            }
        }
        if !replaced {
            out.push_str(seg);
        }
    }
    out
}

fn dist_js_files(dist_client: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dist_client.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dist dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "js") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Every `/assets/...` URL appearing as a double-quoted string (script srcs,
/// link hrefs, and the inline import map body).
fn quoted_asset_urls(text: &str) -> BTreeSet<String> {
    text.split('"')
        .filter_map(|seg| {
            seg.strip_prefix("/assets/")
                .map(|rest| format!("/assets/{rest}"))
        })
        .collect()
}

fn resolve_relative(importer: &str, target: &str) -> String {
    let base = importer.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
    format!("{base}/{target}")
}

/// The set of chunks a browser actually loads: every URL referenced from the
/// HTML (script srcs + inline import map), then every relative import target
/// of those chunks, transitively.
fn fetched_closure_urls(dist_client: &Path, html: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = quoted_asset_urls(html).into_iter().collect();
    while let Some(url) = queue.pop() {
        if !seen.insert(url.clone()) {
            continue;
        }
        if !url.ends_with(".js") {
            continue;
        }
        let path = dist_client.join(url.trim_start_matches('/'));
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{url} must exist on disk at {}: {err}", path.display()));
        for target in body.split('"').filter_map(|seg| seg.strip_prefix("./")) {
            queue.push(resolve_relative(&url, target));
        }
    }
    seen
}

#[test]
fn dist_assets_are_readable_sources_while_paused() {
    // PAUSED (deka#881 DECIDE-1): dist no longer prunes or minifies. Pin what
    // the paused pipeline guarantees: the counter page emits its client
    // chunks, and every shipped `ui/*` chunk is byte-identical to the
    // readable deka_ui source (SSR producers included). The deka#750 byte
    // budgets and producer-absence assertions return with the dist
    // optimization stage.
    let project = tempfile::tempdir().expect("tempdir");
    write_counter_project(project.path());
    let (success, combined) = run_build(project.path());
    assert!(success, "deka build failed: {combined}");

    let dist_client = project.path().join("dist").join("client");
    let files = dist_js_files(&dist_client);
    assert!(
        files.len() >= 5,
        "counter page must emit several client chunks: {files:?}"
    );

    let ui_dir = dist_client.join("assets").join("ui");
    for spec in deka_ui::SPECIFIERS {
        let source = if *spec == "ui/server" {
            continue;
        } else {
            deka_ui::source_for(spec).expect("ui source")
        };
        let stem = deka_ui::file_name_for(spec)
            .expect("ui file name")
            .trim_end_matches(".js");
        let chunk = find_hashed_asset(&ui_dir, stem, "js")
            .unwrap_or_else(|| panic!("dist must ship ui/{stem} under a hashed name"));
        let body = fs::read_to_string(&chunk).expect("read ui chunk");
        // Sibling imports are rewritten to hashed names on disk; normalize
        // those back before comparing against the raw deka_ui source.
        let normalized = strip_sibling_hashes(&body);
        assert!(
            normalized == source,
            "dist ui/{stem} must be the readable source while optimization is paused"
        );
    }

    // The browser-fetched closure must still resolve entirely on disk.
    let html = fs::read_to_string(dist_client.join("index.html")).expect("read dist html");
    let closure = fetched_closure_urls(&dist_client, &html);
    assert!(
        closure.iter().any(|url| url.contains("islands-load.")),
        "fetched closure must include the islands entry: {closure:?}"
    );
}

/// Extracts `value` from the first `attr="value"` occurrence in `html`.
fn attr_value(html: &str, attr: &str) -> String {
    let needle = format!("{attr}=\"");
    let start = html
        .find(&needle)
        .unwrap_or_else(|| panic!("{attr} must appear in dist html"));
    let rest = &html[start + needle.len()..];
    let end = rest.find('"').expect("terminated attribute value");
    rest[..end].to_string()
}

/// Extracts the body of the `<!--deka-island ...-->` comment surrounding the
/// element with `data-deka-id` containing `id_part`: the start comment is
/// searched backwards from that element, the end comment forwards.
fn island_markers_around(html: &str, id_part: &str) -> (String, String) {
    let id = attr_value_containing(html, "data-deka-id", id_part);
    let at = html.find(&id).expect("island element id located above");
    let start_at = html[..at]
        .rfind("deka-island start:")
        .expect("start comment before island element");
    let start_rest = &html[start_at..];
    let start_end = start_rest.find("-->").expect("terminated start comment");
    let start = start_rest[..start_end].trim().to_string();
    let end_rest = &html[at..];
    let end_at = end_rest
        .find("deka-island end:")
        .expect("end comment after island element");
    let end_rest = &end_rest[end_at..];
    let end_end = end_rest.find("-->").expect("terminated end comment");
    let end = end_rest[..end_end].trim().to_string();
    (start, end)
}

/// The `attr="value"` occurrence whose value contains `part`.
fn attr_value_containing(html: &str, attr: &str, part: &str) -> String {
    let needle = format!("{attr}=\"");
    let mut from = 0;
    while let Some(at) = html[from..].find(&needle) {
        let rest = &html[from + at + needle.len()..];
        let end = rest.find('"').expect("terminated attribute value");
        let value = &rest[..end];
        if value.contains(part) {
            return value.to_string();
        }
        from += at + needle.len() + end + 1;
    }
    panic!("{attr} containing {part} must appear in dist html");
}

#[test]
fn dist_island_hydrates_and_counter_increments() {
    let project = tempfile::tempdir().expect("tempdir");
    write_counter_project(project.path());
    let (success, combined) = run_build(project.path());
    assert!(success, "deka build failed: {combined}");

    let dist_client = project.path().join("dist").join("client");
    let html = fs::read_to_string(dist_client.join("index.html")).expect("read dist html");
    let entry_name = html
        .find("islands-load.")
        .map(|at| {
            let rest = &html[at..];
            let end = rest.find('"').expect("terminated entry src");
            rest[..end].to_string()
        })
        .expect("dist html must reference the hashed islands-load chunk");
    assert!(
        entry_name.starts_with("islands-load.") && entry_name.ends_with(".js"),
        "entry script must be the hashed islands-load chunk: {entry_name}"
    );
    let (start, end) = island_markers_around(&html, "Counter");
    let deka_id = attr_value_containing(&html, "data-deka-id", "Counter");

    // Minimal DOM stub around the real SSR'd island markers: comment pair +
    // button carrying the SSR'd label, exactly what hydrate() walks.
    let harness = HYNESS_TEMPLATE
        .replace("__START_MARKER__", &start)
        .replace("__END_MARKER__", &end)
        .replace("__DEKA_ID__", &deka_id)
        .replace("__ENTRY_NAME__", &entry_name);
    fs::write(project.path().join("hydration-harness.js"), harness).expect("write harness");

    let output = Command::new(cli_bin())
        .args(["run", "hydration-harness.js"])
        .current_dir(project.path())
        .output()
        .expect("run hydration harness");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success() && combined.contains("HYDRATION-OK"),
        "hydration harness failed: {combined}"
    );
    assert!(
        combined.contains("1 -> 2 -> 3"),
        "counter must increment 1 -> 2 -> 3 across two clicks: {combined}"
    );
}

#[test]
fn dev_assets_stay_readable_and_unpruned() {
    // Dev flavor (.cache via `deka serve`) must keep the readable,
    // un-pruned sources: the SSR producers survive and the chunk is not
    // minified. Dist matches it byte-for-byte while optimization is paused
    // (deka#881 DECIDE-1); dist parity is pinned by
    // dist_assets_are_readable_sources_while_paused.
    let serve = spawn_counter_serve();
    let http = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build http client");
    let base = format!("http://127.0.0.1:{}", serve.port);
    let html = http
        .get(format!("{base}/"))
        .send()
        .expect("GET /")
        .text()
        .expect("read served html");

    let marker_url = quoted_asset_urls(&html)
        .into_iter()
        .find(|url| url.contains("/assets/ui/island-marker."))
        .expect("served html must reference the hashed island-marker chunk");
    let body = http
        .get(format!("{base}{marker_url}"))
        .send()
        .expect("GET island-marker")
        .text()
        .expect("read island-marker");
    for symbol in PRODUCER_SYMBOLS {
        assert!(
            body.contains(symbol),
            "dev island-marker chunk must keep SSR producer {symbol} (readable, unpruned)"
        );
    }
    assert!(
        body.contains("export function parseIslandMarker"),
        "dev chunk must stay readable (no minification): {body}"
    );
}

struct Serve {
    child: Child,
    port: u16,
    _root: TempDir,
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn spawn_counter_serve() -> Serve {
    let root = tempfile::tempdir().expect("tempdir");
    write_counter_project(root.path());
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
        _root: root,
    };
    let http = Client::new();
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if let Ok(res) = http.get(format!("http://127.0.0.1:{port}/")).send() {
            if res.status().as_u16() == 200 {
                return serve;
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!(
        "deka serve did not become ready on port {port}\nserve.log:\n{}",
        fs::read_to_string(&log_path).unwrap_or_default()
    );
}

/// Stub-DOM hydration harness. The island markers, element id, and entry
/// chunk name are injected from the real built HTML so the test tracks any
/// serialization-format change. `deka run` evaluates it with real module
/// loading, so the dist chunks execute exactly as in a browser.
const HYNESS_TEMPLATE: &str = r##"let failures = 0;
function check(name, cond, extra) {
  if (cond) { console.log("ok: " + name); }
  else { failures += 1; console.log("FAIL: " + name + (extra ? " -- " + extra : "")); }
}
class StubNode {
  constructor(nodeType, tag) {
    this.nodeType = nodeType; this.childNodes = []; this.parentNode = null; this.listeners = {};
    if (nodeType === 1) { this.tagName = (tag || "").toUpperCase(); this.attributes = {}; }
    if (nodeType === 8) this._value = "";
    if (nodeType === 3) this._text = "";
  }
  append(child) { child.parentNode = this; this.childNodes.push(child); return child; }
  get nextSibling() {
    if (!this.parentNode) return null;
    const kids = this.parentNode.childNodes; const idx = kids.indexOf(this);
    return idx >= 0 && idx + 1 < kids.length ? kids[idx + 1] : null;
  }
  getAttribute(name) { return this.attributes ? this.attributes[name] ?? null : null; }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  addEventListener(type, fn) { (this.listeners[type] = this.listeners[type] || []).push(fn); }
  dispatch(type) { for (const fn of this.listeners[type] || []) fn({ type, target: this }); }
  get nodeValue() { return this.nodeType === 8 ? this._value : null; }
  get data() { return this.nodeType === 8 ? this._value : null; }
  get textContent() {
    if (this.nodeType === 3) return this._text;
    return this.childNodes.map((c) => c.textContent || "").join("");
  }
  set textContent(v) {
    if (this.nodeType === 3) { this._text = String(v); }
    else { this.childNodes = []; const t = new StubNode(3); t._text = String(v); this.append(t); }
  }
}
const root = new StubNode(9, "#document");
const main = root.append(new StubNode(1, "main"));
const start = main.append(new StubNode(8));
start._value = "__START_MARKER__";
const button = main.append(new StubNode(1, "button"));
button.setAttribute("data-deka-id", "__DEKA_ID__");
const label = button.append(new StubNode(3)); label._text = "1";
const end = main.append(new StubNode(8));
end._value = "__END_MARKER__";
globalThis.document = root;
globalThis.window = globalThis;
try {
  await import("./dist/client/assets/__ENTRY_NAME__");
  check("entry module evaluated", true);
} catch (err) {
  check("entry module evaluated", false, String((err && err.stack) || err));
}
check("island element hydrated", button.__dekaHydrated === true);
const before = label.textContent;
button.dispatch("click");
const after1 = label.textContent;
button.dispatch("click");
const after2 = label.textContent;
check(`click increments label (${before} -> ${after1} -> ${after2})`,
  before === "1" && after1 === "2" && after2 === "3",
  `button text=[${button.textContent}]`);
if (failures > 0) { console.log(`HYDRATION-FAIL ${failures}`); throw new Error(`hydration harness: ${failures} failure(s)`); }
console.log("HYDRATION-OK");
"##;
