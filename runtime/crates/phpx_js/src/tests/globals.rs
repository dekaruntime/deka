use super::*;

// ---- RFD 21: DekaScript globals and unsafe boundary ----

#[test]
fn ds_prelude_emits_deka_global() {
    let source = "const answer = 42;";
    let js = ds_to_js(source).expect("should compile");
    assert!(
        js.contains("globalThis.deka??=deka"),
        "expected deka global install, got:\n{}",
        js
    );
    assert!(
        js.contains("const deka={"),
        "expected deka object literal, got:\n{}",
        js
    );
    assert!(
        js.contains("unsafe:(tryFn,catchFn,finallyFn)=>"),
        "expected deka.unsafe helper, got:\n{}",
        js
    );
    assert!(
        js.contains("panic:(msg)=>"),
        "expected deka.panic helper, got:\n{}",
        js
    );
}

#[test]
fn ds_prelude_emits_unsafe_namespace() {
    let source = "const answer = 42;";
    let js = ds_to_js(source).expect("should compile");
    assert!(
        js.contains("globalThis.unsafe??=__DekaUnsafeGlobals"),
        "expected unsafe namespace install, got:\n{}",
        js
    );
    assert!(
        js.contains("const __DekaUnsafeGlobals="),
        "expected unsafe globals capture, got:\n{}",
        js
    );
    assert!(
        js.contains("fetch:g.fetch"),
        "expected fetch capture, got:\n{}",
        js
    );
    assert!(
        js.contains("JSON:g.JSON"),
        "expected JSON capture, got:\n{}",
        js
    );
}

#[test]
fn ds_prelude_does_not_emit_for_phpx_mode() {
    let source = "$x = 42;";
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        !js.contains("globalThis.deka??=deka"),
        "PHPX should not include DekaScript prelude:\n{}",
        js
    );
    assert!(
        !js.contains("__DekaUnsafeGlobals"),
        "PHPX should not include unsafe globals capture:\n{}",
        js
    );
}

#[test]
fn ds_prelude_wraps_json_when_referenced() {
    let source = "const val = JSON.parse(text);";
    let js = ds_to_js(source).expect("should compile");
    assert!(
        js.contains("globalThis.JSON??=__DekaUnsafeGlobals.JSON"),
        "expected JSON install, got:\n{}",
        js
    );
    assert!(
        js.contains("globalThis.JSON.parse=(text)=>"),
        "expected JSON.parse wrapper, got:\n{}",
        js
    );
    assert!(
        js.contains("globalThis.JSON.stringify=(value,replacer,space)=>"),
        "expected JSON.stringify wrapper, got:\n{}",
        js
    );
    assert!(
        js.contains("return{__error:e}"),
        "expected error-value return in JSON wrapper, got:\n{}",
        js
    );
}

#[test]
fn ds_prelude_wraps_fetch_when_referenced() {
    let source = "const val = fetch(url);";
    let js = ds_to_js(source).expect("should compile");
    assert!(
        js.contains("globalThis.fetch??=(...args)=>"),
        "expected fetch wrapper, got:\n{}",
        js
    );
    assert!(
        js.contains(".then((r)=>({__ok:r}))"),
        "expected Ok result wrapper, got:\n{}",
        js
    );
    assert!(
        js.contains(".catch((e)=>({__error:e}))"),
        "expected Err result wrapper, got:\n{}",
        js
    );
}

#[test]
fn ds_prelude_does_not_emit_unused_safe_globals() {
    let source = "const answer = 42;";
    let js = ds_to_js(source).expect("should compile");
    assert!(
        !js.contains("globalThis.JSON??="),
        "JSON wrapper should be demand-driven:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.fetch??="),
        "fetch wrapper should be demand-driven:\n{}",
        js
    );
}

#[test]
fn ds_unsafe_block_has_runtime_helper() {
    let source = "const val = unsafe { JSON.parse(text) };";
    let js = ds_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.unsafe"),
        "expected deka.unsafe call, got:\n{}",
        js
    );
    assert!(
        js.contains("globalThis.deka??=deka"),
        "expected deka prelude for unsafe block, got:\n{}",
        js
    );
}
