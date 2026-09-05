//! Regression coverage for deka#51: server-rendered JSX values stay text.
//!
//! These assertions intentionally inspect the rendered HTML, rather than the
//! implementation of the escaping helper. If escaping is removed from the
//! render path, the exact XSS payload below makes this test fail.

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
    let specifier = ModuleSpecifier::parse("file:///deka_ui/render_escape_test.js")
        .expect("parse render escape test module");
    let module_id = runtime
        .load_side_es_module_from_code(&specifier, ModuleCodeString::from(script.to_string()))
        .await
        .expect("load render escape test module");
    let evaluation = runtime.mod_evaluate(module_id);
    runtime
        .run_event_loop(deno_core::PollEventLoopOptions::default())
        .await
        .expect("run render escape test module");
    evaluation.await.expect("evaluate render escape test module");
}

#[tokio::test(flavor = "current_thread")]
async fn server_emits_escaped_text_attributes_and_fragment_arrays() {
    let base = deno_core::ModuleSpecifier::from_directory_path(js_dir())
        .expect("js dir file url")
        .to_string();
    let script = format!(
        r#"
import {{ renderToString, renderToStringAsync }} from "{base}/server.js";
import {{ jsx, jsxs, Fragment }} from "{base}/jsx.js";

function assertEqual(actual, expected, label) {{
  if (actual !== expected) throw new Error(label + ": got " + actual + ", expected " + expected);
}}

const text = '<script>alert(1)</script>';
const attr = '" onerror=';
function PoC(props) {{
  return jsxs('div', {{ title: props.attr }}, [
    props.text,
    jsx('span', {{}}, props.text),
  ]);
}}

const expected = '<div title="&quot; onerror=">&lt;script&gt;alert(1)&lt;/script&gt;<span>&lt;script&gt;alert(1)&lt;/script&gt;</span></div>';
const rendered = renderToString(jsx(PoC, {{ text, attr }}));
assertEqual(rendered.html, expected, 'component PoC HTML');
const allCharacters = '& < > " ' + String.fromCharCode(39);
assertEqual(
  renderToString(jsx('p', {{ title: allCharacters }}, allCharacters)).html,
  '<p title="&amp; &lt; &gt; &quot; &#39;">&amp; &lt; &gt; &quot; &#39;</p>',
  'all HTML-sensitive characters',
);

assertEqual(
  renderToString(jsx(Fragment, {{}}, [text, attr])).html,
  '&lt;script&gt;alert(1)&lt;/script&gt;&quot; onerror=',
  'fragment array HTML',
);

function ReturnsString() {{ return text; }}
assertEqual(
  renderToString(jsx(ReturnsString, {{}})).html,
  '&lt;script&gt;alert(1)&lt;/script&gt;',
  'component-returned string HTML',
);

const asyncResult = await renderToStringAsync(jsx(PoC, {{ text, attr }}));
assertEqual(asyncResult.html, expected, 'async component PoC HTML');
"#
    );
    run_driver(&script).await;
}
