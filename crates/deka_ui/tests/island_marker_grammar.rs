//! deka#622 finding C: the `deka-island` comment grammar has exactly one
//! definition (crates/deka_ui/js/island-marker.js), imported by both the
//! producer (ui/server) and the consumer (ui/client). These tests are the
//! drift guard: they feed the markup ui/server actually emits through the
//! parser ui/client actually uses, field for field. Before the unification
//! this suite did not exist — a consumer regex that rejected every marker
//! the server emitted would still pass the whole test suite while hydrate
//! silently no-opped.

use std::path::PathBuf;
use std::rc::Rc;

use deno_core::{JsRuntime, ModuleCodeString, ModuleSpecifier, RuntimeOptions};

fn js_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("js")
}

/// jsx/router/form/suspense are DekaScript ports; their pinned emit lives in
/// emit/ (deka#771). The remaining modules are still hand-written JavaScript.
fn emit_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("emit")
}

fn js_source(name: &str) -> String {
    std::fs::read_to_string(js_dir().join(name)).expect("read ui js source")
}

/// Materialize every ui module into one directory, the same sibling layout
/// the runtime writes (pool's `materialize_ui_module`): server.js resolves
/// its relative `./jsx.js` import against the directory it lives in.
fn materialize_ui_modules() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("deka_ui_modules_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create ui module dir");
    for name in ["server.js", "client.js", "island-marker.js", "reactive.js"] {
        std::fs::copy(js_dir().join(name), dir.join(name)).expect("copy ui js source");
    }
    for name in ["jsx.js", "router.js", "form.js", "suspense.js"] {
        std::fs::copy(emit_dir().join(name), dir.join(name)).expect("copy ui emit");
    }
    dir
}

async fn run_driver(script: &str) {
    let mut runtime = JsRuntime::new(RuntimeOptions {
        module_loader: Some(Rc::new(deno_core::FsModuleLoader)),
        ..Default::default()
    });
    let specifier = ModuleSpecifier::parse("file:///deka_ui/island_marker_grammar_test.js")
        .expect("parse test module specifier");
    let module_id = runtime
        .load_side_es_module_from_code(&specifier, ModuleCodeString::from(script.to_string()))
        .await
        .expect("load island marker grammar test module");
    let evaluation = runtime.mod_evaluate(module_id);
    runtime
        .run_event_loop(deno_core::PollEventLoopOptions::default())
        .await
        .expect("run island marker grammar test module");
    evaluation
        .await
        .expect("evaluate island marker grammar test module");
}

/// Behavioural half of the falsification check: real ui/server emission,
/// real shared parser, field for field.
#[tokio::test(flavor = "current_thread")]
async fn server_emitted_markers_parse_field_for_field() {
    let base = deno_core::ModuleSpecifier::from_directory_path(materialize_ui_modules())
        .expect("ui modules file url")
        .to_string();
    let script = format!(
        r#"
import {{ renderToString }} from "{base}/server.js";
import {{ jsx }} from "{base}/jsx.js";
import {{ parseIslandMarker, formatIslandStart, formatIslandEnd }} from "{base}/island-marker.js";
import * as client from "{base}/client.js";

function assert(cond, msg) {{
  if (!cond) throw new Error("island grammar: " + msg);
}}
function same(a, b) {{ return JSON.stringify(a) === JSON.stringify(b); }}
function extractStart(html) {{
  const m = html.match(/<!--(deka-island start:[^-]*)-->/);
  assert(m, "start marker present in: " + html);
  return m[1];
}}

// ui/client must load with its real import graph (jsx, reactive, island-marker).
assert(typeof client.hydrate === "function", "client.js loads");
assert(typeof client.registerIsland === "function", "client.js loads");

// 1. client:* island: the marker ui/server wraps around a directive island.
function Widget() {{ return jsx("span", null, "w"); }}
const island = renderToString(jsx(Widget, {{ "client:load": true, label: "hi", count: 2 }}));
const parsed = parseIslandMarker(extractStart(island.html));
assert(parsed !== null, "island marker parses");
assert(parsed.name === "Widget", "island name, got " + parsed.name);
assert(parsed.directive === "load", "island directive, got " + parsed.directive);
assert(same(parsed.props, {{ label: "hi", count: 2 }}), "island props, got " + JSON.stringify(parsed.props));
assert(parsed.id === "" && parsed.enc === "" && parsed.cache === "", "island optional fields empty");
assert(island.html.includes("<!--" + formatIslandEnd("Widget") + "-->"), "paired end marker");

// 2. server:defer island: id/cache present, props absent. No defer secret is
// bound in this isolate, so enc is omitted exactly as in production SSR.
function DeferComp() {{ return jsx("b", null, "d"); }}
const deferred = renderToString(
  jsx(DeferComp, {{ "server:defer": true, cache: "public, max-age=30" }},
    [jsx("span", {{ "data-deka-id": "fb", slot: "fallback" }}, ".")])
);
const dparsed = parseIslandMarker(extractStart(deferred.html));
assert(dparsed !== null, "defer marker parses");
assert(dparsed.name === "DeferComp", "defer name");
assert(dparsed.directive === "defer", "defer directive");
assert(dparsed.id !== "" && /^D:\d+$/.test(dparsed.id), "defer id, got " + dparsed.id);
assert(dparsed.cache === "public, max-age=30", "defer cache, got " + dparsed.cache);
assert(same(dparsed.props, {{}}), "defer props absent");
assert(dparsed.enc === "", "defer enc omitted without a bound secret");

// 3. Formatter round-trip with every optional field set, exact wire bytes.
const wire = formatIslandStart({{
  name: "A",
  directive: "idle",
  propsJson: "{{\"k\":1}}",
  id: "D:7",
  enc: "QUJD",
  cache: "no-store",
}});
assert(
  wire === "deka-island start:QQ== directive:aWRsZQ== props:eyJrIjoxfQ== id:RDo3 enc:QUJD cache:bm8tc3RvcmU=",
  "exact wire format, got " + wire
);
const round = parseIslandMarker(wire);
assert(round !== null && round.name === "A" && round.directive === "idle"
  && same(round.props, {{ k: 1 }}) && round.id === "D:7"
  && round.enc === "QUJD" && round.cache === "no-store", "round trip field for field");

// 4. Field order is part of the grammar: out-of-order and unknown fields
// must not parse (the old consumer regex silently grew a `mac:` alternative
// no producer ever emitted — deka#622 finding C).
assert(parseIslandMarker("deka-island start:QQ== directive:bG9hZA==") !== null, "minimal marker parses");
assert(parseIslandMarker("deka-island start:QQ== props:e30= directive:bG9hZA==") === null, "reordered fields rejected");
assert(parseIslandMarker("deka-island start:QQ== directive:bG9hZA== mac:e30=") === null, "unknown field rejected");
assert(parseIslandMarker("deka-island start:QQ== directive:bG9hZA== cache:bm8t store") === null, "space in value rejected");
assert(parseIslandMarker("just a comment") === null, "plain comment rejected");
assert(parseIslandMarker(null) === null, "null rejected");
"#
    );
    run_driver(&script).await;
}

/// Static half: both sides must derive from the shared module. A corrected
/// second regex spelled out in client.js (or a re-inlined template in
/// server.js) is the original bug with better spelling, so it must not be
/// reachable — any grammar text outside island-marker.js fails here.
#[test]
fn grammar_is_defined_in_exactly_one_place() {
    let server = js_source("server.js");
    let client = js_source("client.js");
    let marker = js_source("island-marker.js");

    for side in [&server, &client] {
        assert!(
            side.contains("from \"./island-marker.js\""),
            "both sides import the shared grammar module"
        );
    }
    assert!(
        !server.contains("deka-island start:") && !server.contains("directive:"),
        "server.js must not spell the marker template inline"
    );
    assert!(
        !client.contains("directive:"),
        "client.js must not spell the marker regex inline"
    );
    assert!(
        !client.contains("function decodeB64"),
        "client.js must not re-localize the base64 decoder"
    );
    assert!(
        client.contains("function liveText"),
        "client.js hydrate walk depends on liveText"
    );
    assert!(
        !client.contains("mac:"),
        "the dead mac field must not return to the comment grammar"
    );

    // The one grammar description both sides derive from.
    assert!(
        marker.contains("ISLAND_MARKER_TAG = \"deka-island\""),
        "marker tag is defined in the shared module"
    );
    assert!(
        marker.contains(
            "ISLAND_MARKER_FIELDS = [\"start\", \"directive\", \"props\", \"id\", \"enc\", \"cache\"]"
        ),
        "field list is defined once, in order, in the shared module"
    );
}

/// The Rust scanners (crates/http/src/websocket.rs, the HMR patch client)
/// match only the literal `deka-island start:` / `deka-island end:` prefixes;
/// pin the shared module's tag to what they needle on.
#[test]
fn coarse_scanners_share_the_marker_prefix() {
    let http_ws = {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest
            .parent()
            .expect("workspace member")
            .parent()
            .expect("workspace root")
            .join("crates/http/src/websocket.rs")
    };
    let scanner = std::fs::read_to_string(http_ws).expect("read websocket.rs");
    let marker = js_source("island-marker.js");
    assert!(
        marker.contains("ISLAND_MARKER_TAG = \"deka-island\""),
        "shared module still defines the tag"
    );
    assert!(
        scanner.contains("<!--deka-island start:") && scanner.contains("<!--deka-island end:"),
        "coarse scanner needles still match the shared tag"
    );
}
