use super::*;

#[test]
fn jsx_element_compiles() {
    let source = "function View(): Object { return <div class=\"test\">hello</div>; }";
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("div") || js.contains("jsx") || js.contains("h("),
        "expected JSX output, got:\n{}",
        js
    );
}

// ==== Phase 2: compile-time rewrite regression tests ====
#[test]
fn jsx_basic_element() {
    let js =
        phpx_to_js("function View(): Object { return <div>hello</div>; }").expect("should compile");
    assert!(js.contains("div"), "expected div element, got:\n{}", js);
}

#[test]
fn jsx_with_props() {
    let js = phpx_to_js(
        "function View(): Object { return <div class=\"test\" id=\"main\">content</div>; }",
    )
    .expect("should compile");
    assert!(
        js.contains("test") && js.contains("main"),
        "expected props in JSX output, got:\n{}",
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
        js.contains("name"),
        "expected expression child, got:\n{}",
        js
    );
}

#[test]
fn jsx_self_closing() {
    let js = phpx_to_js("function View(): Object { return <br />; }").expect("should compile");
    assert!(js.contains("br"), "expected br element, got:\n{}", js);
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
        js.contains("div") && js.contains("span"),
        "expected nested elements, got:\n{}",
        js
    );
}

// ---- Scope validation edge cases ----
