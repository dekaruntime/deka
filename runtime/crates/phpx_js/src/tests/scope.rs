use super::*;

#[test]
fn scope_outer_declare_inner_assign_no_warning() {
    // Variable declared at outer scope, assigned inside foreach — no warning.
    // Wrapped in a function to avoid top-level globalThis mirroring.
    let source = "function test(): int {\n  $total = 0;\n  foreach ([1,2,3] as $v) {\n    $total = $total + $v;\n  }\n  return $total;\n}";
    let (js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    let scope_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("first assigned inside a block"))
        .collect();
    assert!(
        scope_warnings.is_empty(),
        "expected no scope warnings, got: {:?}",
        scope_warnings
    );
    assert!(
        js.contains("let total = 0"),
        "expected let total at function scope, got:\n{}",
        js
    );
    // Inside the function, total should never become globalThis.total
    assert!(
        !js.contains("globalThis.total"),
        "total should not be globalThis inside function, got:\n{}",
        js
    );
}

#[test]
fn scope_first_assign_inside_block_warns() {
    // Variable first assigned inside a foreach, then read outside — should warn.
    let source = "function test(): int {\n  foreach ([1,2,3] as $v) {\n    $total = $total + $v;\n  }\n  return $total;\n}";
    let (_js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("$total") && w.contains("first assigned inside a block")),
        "expected scope warning for $total, got: {:?}",
        warnings
    );
}

#[test]
fn scope_normal_block_scoped_no_warning() {
    // Variables used only within their declaring scope — no warning.
    let source = "function test(): int {\n  $x = 10;\n  if ($x > 5) {\n    $y = $x + 1;\n  }\n  return $x;\n}";
    let (_js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    let scope_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("first assigned inside a block"))
        .collect();
    assert!(
        scope_warnings.is_empty(),
        "expected no scope warnings, got: {:?}",
        scope_warnings
    );
}

// ==================================================================
// Phase 4: Comprehensive PHPX -> JS snapshot coverage
// ==================================================================

// ---- Variables and assignments ----

#[test]
fn scope_multiple_nested_blocks() {
    let source = r#"
function f(): int {
$total = 0;
foreach ([1, 2] as $v) {
    if ($v > 0) {
        $total = $total + $v;
    }
}
return $total;
}
"#;
    let (_js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    let scope_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("first assigned inside a block"))
        .collect();
    assert!(
        scope_warnings.is_empty(),
        "expected no scope warnings for outer-declared var, got: {:?}",
        scope_warnings
    );
}

#[test]
fn scope_if_block_declaration_warns_on_outside_use() {
    let source = r#"
function f(): int {
if (true) {
    $inner = 42;
}
return $inner;
}
"#;
    let (_js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("$inner") && w.contains("first assigned inside a block")),
        "expected warning for $inner used outside if block, got: {:?}",
        warnings
    );
}

// ---- Unset ----

