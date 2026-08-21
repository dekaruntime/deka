use super::*;

#[test]
fn jsx_element_compiles() {
    let source = "function View(): Object { return <div class=\"test\">hello</div>; }";
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.ui.jsx"),
        "expected deka.ui.jsx output, got:\n{}",
        js
    );
}

// ==== Phase 2: compile-time rewrite regression tests ====
#[test]
fn jsx_basic_element() {
    let js =
        phpx_to_js("function View(): Object { return <div>hello</div>; }").expect("should compile");
    assert!(
        js.contains("deka.ui.jsx(\"div\", {\"children\": \"hello\"})"),
        "expected deka.ui.jsx call for basic element, got:\n{}",
        js
    );
}

#[test]
fn jsx_static_children_use_jsxs() {
    let source = r#"
function View(): Object {
  return <div><span>one</span><span>two</span></div>;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.ui.jsxs(\"div\", {\"children\": [deka.ui.jsx(\"span\", {\"children\": \"one\"}), deka.ui.jsx(\"span\", {\"children\": \"two\"})]})") ,
        "expected deka.ui.jsxs call for multiple static children, got:\n{}",
        js
    );
}

#[test]
fn jsx_with_props() {
    let js = phpx_to_js(
        "function View(): Object { return <div class=\"test\" id=\"main\">content</div>; }",
    )
    .expect("should compile");
    assert!(
        js.contains("deka.ui.jsx(\"div\", {\"class\": \"test\", \"id\": \"main\", \"children\": \"content\"})"),
        "expected props in deka.ui.jsx output, got:\n{}",
        js
    );
}

#[test]
fn jsx_with_expression_child() {
    let source = r#"
function View(): Object {
$name = 'world';
return <div>hello {$name}</div>;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.ui.jsxs(\"div\", {\"children\": [\"hello \", name]})"),
        "expected expression child in deka.ui.jsxs output, got:\n{}",
        js
    );
}

#[test]
fn jsx_self_closing() {
    let js = phpx_to_js("function View(): Object { return <br />; }").expect("should compile");
    assert!(
        js.contains("deka.ui.jsx(\"br\", {})")
            || js.contains("deka.ui.jsx(\"br\", )"),
        "expected deka.ui.jsx call for self-closing element, got:\n{}",
        js
    );
}

#[test]
fn jsx_nested_elements() {
    let source = r#"
function View(): Object {
return <div><span>inner</span></div>;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.ui.jsx(\"div\", {\"children\": deka.ui.jsx(\"span\", {\"children\": \"inner\"})})"),
        "expected nested deka.ui.jsx calls, got:\n{}",
        js
    );
}

#[test]
fn jsx_function_component_tag() {
    let source = r#"
function Greeting($props: object): object {
  return <h1>Hello {$props.name}</h1>;
}
function View(): Object {
  return <Greeting name="world" />;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.ui.jsx(Greeting, {\"name\": \"world\"})"),
        "expected function component passed as identifier to deka.ui.jsx, got:\n{}",
        js
    );
}

#[test]
fn jsx_fragment_uses_deka_ui_fragment() {
    let source = r#"
function View(): Object {
  return <><span>one</span><span>two</span></>;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.ui.Fragment"),
        "expected fragment to emit deka.ui.Fragment, got:\n{}",
        js
    );
    assert!(
        js.contains("deka.ui.jsxs(deka.ui.Fragment, {\"children\": [deka.ui.jsx(\"span\", {\"children\": \"one\"}), deka.ui.jsx(\"span\", {\"children\": \"two\"})]})"),
        "expected fragment to emit deka.ui.jsxs(deka.ui.Fragment, ...), got:\n{}",
        js
    );
}

#[test]
fn jsx_spread_props() {
    let source = r#"
fn View(): Object {
  const rest = { class: "x" };
  return <div {...rest} id="main" />;
}
"#;
    let js = ds_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.ui.jsx(\"div\", {...rest, \"id\": \"main\"})")
            || js.contains("deka.ui.jsx(\"div\", {...rest,\"id\":\"main\"})"),
        "expected spread props in deka.ui.jsx output, got:\n{}",
        js
    );
}

#[test]
fn jsx_event_handler_prop() {
    let source = r#"
fn View(): Object {
  return <button onClick={fn() { print("clicked") }}>click me</button>;
}
"#;
    let js = ds_to_js(source).expect("should compile");
    assert!(
        js.contains("deka.ui.jsx(\"button\", {\"onClick\": function()")
            || js.contains("deka.ui.jsxs(\"button\", {\"onClick\": function()"),
        "expected event handler prop in deka.ui.jsx/jsxs output, got:\n{}",
        js
    );
}

// ---- Scope validation edge cases ----
