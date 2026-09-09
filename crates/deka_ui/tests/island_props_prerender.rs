//! deka#746 F3: island props during server render, and loud live() errors.
//!
//! Props cross the island boundary in-process during the server pass — the
//! renderer invokes the component with the same props object it serializes
//! into the marker — so the initial HTML must contain the prop values
//! (blank-then-fill used to be the silent default when a prop was absent:
//! the read threw inside `live()`, the renderer swallowed it, and the node
//! rendered empty).

use std::path::PathBuf;
use std::rc::Rc;

use deno_core::{JsRuntime, ModuleCodeString, ModuleSpecifier, RuntimeOptions};

fn js_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("js")
}

async fn run_driver(script: &str) {
    let mut runtime = JsRuntime::new(RuntimeOptions {
        module_loader: Some(Rc::new(deno_core::FsModuleLoader)),
        ..Default::default()
    });
    let specifier = ModuleSpecifier::parse("file:///deka_ui/island_props_test.js")
        .expect("parse island props test module");
    let module_id = runtime
        .load_side_es_module_from_code(&specifier, ModuleCodeString::from(script.to_string()))
        .await
        .expect("load island props test module");
    let evaluation = runtime.mod_evaluate(module_id);
    runtime
        .run_event_loop(deno_core::PollEventLoopOptions::default())
        .await
        .expect("run island props test module");
    evaluation.await.expect("evaluate island props test module");
}

#[tokio::test(flavor = "current_thread")]
async fn island_props_render_into_initial_html_sync_and_async() {
    let base = deno_core::ModuleSpecifier::from_directory_path(js_dir())
        .expect("js dir file url")
        .to_string();
    let script = format!(
        r#"
import {{ renderToString, renderToStringAsync }} from "{base}/server.js";
import {{ jsx }} from "{base}/jsx.js";
import {{ live }} from "{base}/reactive.js";
import {{ formatIslandStart, formatIslandEnd }} from "{base}/island-marker.js";

function assertEqual(actual, expected, label) {{
  if (actual !== expected) throw new Error(label + ": got " + actual + ", expected " + expected);
}}

// deka#746 F3 fixture: a struct-typed island prop read through a live()
// expression. The server pass holds the real value, so `Ada`/`36` must be
// in the initial HTML — empty spans mean the props never arrived.
function Card(props) {{
  return jsx('div', {{}}, [
    jsx('span', {{ id: 'nm' }}, live(function() {{ return props.user.name; }})),
    jsx('span', {{ id: 'ag' }}, live(function() {{ return props.user.age; }})),
  ]);
}}
const user = {{ name: 'Ada', age: 36 }};
const expected = '<div><span id="nm">Ada</span><span id="ag">36</span></div>';
const markerAndHtml = '<!--' + formatIslandStart({{ name: 'Card', directive: 'load', propsJson: JSON.stringify({{ user }}) }}) + '-->'
  + expected + '<!--' + formatIslandEnd('Card') + '-->';
assertEqual(
  renderToString(jsx(Card, {{ user, "client:load": true }})).html,
  markerAndHtml,
  'sync island props HTML'
);
const asyncResult = await renderToStringAsync(jsx(Card, {{ user, "client:load": true }}));
assertEqual(asyncResult.html, markerAndHtml, 'async island props HTML');
"#
    );
    run_driver(&script).await;
}

#[tokio::test(flavor = "current_thread")]
async fn throwing_live_expression_fails_render_instead_of_empty_node() {
    let base = deno_core::ModuleSpecifier::from_directory_path(js_dir())
        .expect("js dir file url")
        .to_string();
    let script = format!(
        r#"
import {{ renderToString, renderToStringAsync }} from "{base}/server.js";
import {{ jsx }} from "{base}/jsx.js";
import {{ live }} from "{base}/reactive.js";

function assertEqual(actual, expected, label) {{
  if (actual !== expected) throw new Error(label + ": got " + actual + ", expected " + expected);
}}

// A live() expression that reads a field of an absent prop THROWS. That is a
// render error, not an empty node: the renderer must surface it so a build
// fails loudly instead of shipping blank-then-fill HTML (deka#746 F3).
function Card(props) {{
  return jsx('span', {{ id: 'nm' }}, live(function() {{ return props.user.name; }}));
}}

let syncThrew = false;
try {{
  renderToString(jsx(Card, {{ user: undefined, "client:load": true }}));
}} catch (err) {{
  syncThrew = /Cannot read propert/.test(String(err));
}}
assertEqual(syncThrew, true, 'sync render must throw on absent prop');

let asyncThrew = false;
try {{
  await renderToStringAsync(jsx(Card, {{ user: undefined, "client:load": true }}));
}} catch (err) {{
  asyncThrew = /Cannot read propert/.test(String(err));
}}
assertEqual(asyncThrew, true, 'async render must reject on absent prop');
"#
    );
    run_driver(&script).await;
}
