// RFD 13 conformance gate #1 — diagnostic snapshots.
//
// RFD 13 principle under test: "Diagnostics are part of the language. A
// misleading error message is a defect." These tests pin down exactly what
// `compile_phpx_source_to_js` says TODAY for a fixed set of `.ds` inputs. A
// test here fails the moment a message text changes — which is the point:
// nobody should be able to silently reword (or silently regress) a
// diagnostic. When a fix lands for one of the `KNOWN-BAD` cases below, its
// assertion must be updated to the corrected text in the same PR that fixes
// it — that diff is the proof the fix landed, and it is a welcome one.
//
// One defect is already filed and is expected to still be present:
//   - #50: `(n) => n * 2` (JS/TS arrow syntax) is reported as "Missing
//     semicolon" instead of naming the real cause (arrow functions/JS arrow
//     syntax are not accepted; use `fn(...) => ...`).
//
// `enum Color { Red, Green }` is a third symptom of the "Missing semicolon"
// misdirection class (RFD 9/RFD 10 syntax migration, referenced from #50):
// the JS-style enum body isn't accepted yet, and the parser reports it as a
// punctuation problem rather than an unsupported-syntax problem.

/// Compile `.ds` source through the same entry point the CLI's `transpile`
/// and `run` commands use, and return the formatted diagnostic string on
/// failure (this is exactly what a `.ds` author sees on screen today).
fn ds_diagnostic(source: &str) -> Result<String, String> {
    crate::compile_phpx_source_to_js(
        source,
        "conformance.ds",
        crate::parse_source_module_meta(source),
    )
}

#[test]
fn snapshot_null_literal_is_currently_accepted_without_diagnostic() {
    // NOTE: CLAUDE.md's "No null" rule says `null` literals should be
    // rejected in PHPX/DekaScript. Today, in `.ds` mode, a bare `null`
    // literal binding compiles clean with no diagnostic at all. This is a
    // real gap (not one of the two defects this gate was built to track),
    // recorded here so a future gate change is visible as a diff, not a
    // surprise.
    let result = ds_diagnostic("const a = null;");
    let js = result.expect("`const a = null;` currently compiles without a diagnostic");
    assert!(
        js.contains("const a = deka.freeze(null)"),
        "expected the null const binding to be frozen: {js}"
    );
}

#[test]
fn snapshot_null_comparison_rejected_in_dekascript() {
    let err = ds_diagnostic("export function f(a: int): bool { return a == null; }")
        .expect_err("null comparison must be rejected");
    assert!(
        err.contains("Null comparisons are not allowed; use isset() instead"),
        "diagnostic text changed, update this snapshot: {err}"
    );
}

#[test]
fn snapshot_try_catch_rejected_in_dekascript() {
    let err = ds_diagnostic("try { } catch (e) { }").expect_err("try/catch must be rejected");
    assert!(
        err.contains("try/catch is not allowed in DekaScript."),
        "diagnostic text changed, update this snapshot: {err}"
    );
    assert!(
        err.contains("Use Result<T, E> instead of exceptions."),
        "diagnostic text changed, update this snapshot: {err}"
    );
}

#[test]
fn snapshot_throw_rejected_in_dekascript() {
    let err = ds_diagnostic("throw \"boom\";").expect_err("throw must be rejected");
    assert!(
        err.contains("throw is not allowed in DekaScript."),
        "diagnostic text changed, update this snapshot: {err}"
    );
}

#[test]
fn snapshot_class_rejected_in_dekascript() {
    let err = ds_diagnostic("class Foo { }").expect_err("class declarations must be rejected");
    assert!(
        err.contains("Classes are not allowed in DekaScript."),
        "diagnostic text changed, update this snapshot: {err}"
    );
    assert!(
        err.contains("Use structs instead of classes."),
        "diagnostic text changed, update this snapshot: {err}"
    );
}

#[test]
fn snapshot_js_arrow_function_reports_missing_semicolon() {
    // KNOWN-BAD (#50): the real cause is that `(n) => ...` (JS/TS arrow
    // syntax) is unsupported — DekaScript still only accepts `fn(n) => ...`.
    // The parser instead reports a missing semicolon, sending the reader
    // hunting for punctuation that isn't the problem.
    let err = ds_diagnostic("const f = (n) => n * 2;")
        .expect_err("JS-style arrow functions are not accepted yet");
    assert!(
        err.contains("Missing semicolon"),
        "diagnostic text changed — if this now names arrow functions, #50 is fixed: update this snapshot: {err}"
    );
}

#[test]
fn snapshot_struct_field_accepts_bare_identifier_in_dekascript() {
    // dekaruntime/deka#93: DekaScript struct fields and literals use bare
    // identifiers (`x: int` / `x: 3`) instead of the PHP-style `$x` sigil.
    let js = ds_diagnostic("struct Point { x: int; y: int } const p = Point { x: 3, y: 4 };")
        .expect("bare struct field names should be accepted in DekaScript");
    assert!(
        js.contains(r#"const Point = deka.Struct("Point")"#),
        "struct factory missing in emitted JS: {js}"
    );
    assert!(
        js.contains(r#"const p = deka.freeze(Point({"x": 3, "y": 4}))"#),
        "frozen struct literal missing in emitted JS: {js}"
    );
}

#[test]
fn snapshot_enum_js_style_body_is_accepted() {
    // dekaruntime/deka#93: DekaScript enum bodies use bare variant names
    // with optional payloads and commas, instead of PHP-style `case Name;`.
    let js = ds_diagnostic("enum Color { Red, Green }")
        .expect("JS-style enum member list should be accepted in DekaScript");
    assert!(
        js.contains(r#"__enum: "Color""#),
        "enum tag missing in emitted JS: {js}"
    );
    assert!(
        js.contains(r#"__case: "Red""#),
        "Red case missing in emitted JS: {js}"
    );
    assert!(
        js.contains(r#"__case: "Green""#),
        "Green case missing in emitted JS: {js}"
    );
}

#[test]
fn snapshot_enum_payload_match_binds_pattern_variable() {
    // dekaruntime/deka#94: matching an enum variant with a payload should
    // extract the payload into the pattern variable.
    let js = ds_diagnostic(
        r#"enum Option<T> { Some(T), None }
const found = Option.Some("DekaScript");
const message = match (found) {
  Option.Some(value) => value,
  Option.None => "nothing",
  _ => "unknown",
};
console.log(message);"#,
    )
    .expect("enum payload match should compile");
    assert!(
        js.contains(r#"found.__enum === "Option" && found.__case === "Some""#),
        "expected tag-based guard for payload match: {js}"
    );
    assert!(
        js.contains(r#"const value = found["T"];"#),
        "expected payload binding for match pattern: {js}"
    );
}

#[test]
fn snapshot_destructured_object_parameter_compiles() {
    // dekaruntime/deka#95: DekaScript should accept object-destructured
    // function parameters like `{ name }: GreetingProps`.
    let js = ds_diagnostic(
        r#"interface GreetingProps {
  name: string
}
function Greeting({ name }: GreetingProps): string {
  return `Hello ${name}`
}
console.log(Greeting({ name: "DekaScript" }));"#,
    )
    .expect("destructured object parameter should compile");
    assert!(
        js.contains(r#"name = name["name"];"#),
        "expected destructuring prologue in emitted JS: {js}"
    );
}

#[test]
fn snapshot_component_file_with_separator_compiles() {
    // dekaruntime/deka#96: A DekaScript component file may define functions
    // before a single '---' delimiter and put the JSX template after it.
    let js = ds_diagnostic(
        r#"interface GreetingProps {
  name: string
}
function Greeting({ name }: GreetingProps): VNode {
  return <h1>Hello {name}</h1>
}
---
<Greeting name="DekaScript" />"#,
    )
    .expect("component file with script/template separator should compile");
    assert!(
        js.contains(r#"function Greeting(name)"#),
        "expected component function in emitted JS: {js}"
    );
    assert!(
        js.contains(r#"Hello "#),
        "expected rendered greeting in emitted JS: {js}"
    );
}

#[test]
fn snapshot_async_function_return_type_is_diagnosed_correctly() {
    // Not a known-bad case: this diagnostic is accurate and names the real
    // cause (async functions must return `Promise<T>`). Recorded so a
    // regression here — e.g. if it started saying "PHPX" too — is caught.
    let err = ds_diagnostic("export async function f(): int { return 1; }")
        .expect_err("async function must declare a Promise<T> return type");
    assert!(
        err.contains("Async function must declare Promise<T> return type, got int"),
        "diagnostic text changed, update this snapshot: {err}"
    );
}

#[test]
fn snapshot_option_enum_payload_match_compiles() {
    // dekaruntime/deka#93: A generic enum with a payload must be constructible
    // and pattern-matchable in DekaScript.
    let js = ds_diagnostic(
        r#"enum Option<T> {
  Some(T),
  None,
}

const found = Option.Some("DekaScript");

const message = match (found) {
  Option.Some(value) => value,
  Option.None => "nothing",
  _ => "unknown",
};

console.log(message);"#,
    )
    .expect("option enum payload match should compile");
    assert!(
        js.contains("__case"),
        "expected enum case tag in emitted JS: {js}"
    );
}

#[test]
fn snapshot_jsx_component_with_separator_and_destructured_props_compiles() {
    // dekaruntime/deka#93: A DekaScript component file may use an interface
    // typed destructured parameter, define the function before a single '---'
    // delimiter, and use the component in the JSX template after it.
    let js = ds_diagnostic(
        r#"interface GreetingProps {
  name: string
}

function Greeting({ name }: GreetingProps): VNode {
  return <h1>Hello {name}</h1>
}

---
<Greeting name="DekaScript" />"#,
    )
    .expect("jsx component with separator and destructured props should compile");
    assert!(
        js.contains(r#"function Greeting(name)"#),
        "expected component function in emitted JS: {js}"
    );
}
