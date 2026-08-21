// RFD 13 conformance gate #2 — emission budget.
//
// RFD 13 principle under test: "Impose as little runtime overhead as
// possible." Prior to #47, `compile_phpx_source_to_js` emitted an
// unconditional `globalThis.*` prelude on every `.ds` program, whether or
// not the program referenced any of it. For the 4-line `greet`/`print`
// program below, that prelude was ~49 unconditional `globalThis.X ??= ...`
// assignment lines out of ~82 total emitted lines — the program's own code
// was 8 of those 82 lines.
//
// #47 landed: prelude emission is now demand-driven (`JsSubsetEmitter::finish`
// in emitter/mod.rs scans the already-emitted program body for
// `globalThis.<name>` references — see `LEAF_GLOBALS` / `global_deps` /
// `body_refs_global` — and only installs the entries the program actually
// uses). For `BUDGET_SOURCE`, which references none of the mission/runtime
// helpers, that set is now empty: 0 `globalThis.X ??= ...` lines, 15 total
// emitted lines.
//
// Thresholds here are set at CURRENT REALITY PLUS A SMALL MARGIN, not at
// an aspirational target. They exist to catch the budget silently growing
// back — e.g. a new unconditional `out.push_str("globalThis.Y ??= ...")`
// added to `finish()` without a `want(...)` guard.

const BUDGET_SOURCE: &str = r#"fn greet(name: string): string {
    return "hello " + name;
}
export { greet };
print(greet("world"));
"#;

/// Ceiling on total emitted lines for `BUDGET_SOURCE`. Current reality
/// (post-#47): 15 lines. Margin: +5, to absorb incidental
/// whitespace/formatting churn that isn't itself a budget regression.
const MAX_EMITTED_LINES: usize = 20;

/// Ceiling on unconditional `globalThis.<name> ??= ...` prelude assignment
/// lines for `BUDGET_SOURCE`. Current reality (post-#47): 0, since this
/// program references none of the demand-driven mission/runtime helpers.
/// Margin: +2, to allow one incidental new leaf helper without tripping
/// the gate on an unrelated change.
const MAX_GLOBALTHIS_ASSIGNMENTS: usize = 2;

fn compile_budget_source() -> String {
    crate::compile_phpx_source_to_js(
        BUDGET_SOURCE,
        "budget.ds",
        crate::parse_source_module_meta(BUDGET_SOURCE),
    )
    .expect("minimal greet/print program must compile")
}

fn count_globalthis_prelude_assignments(js: &str) -> usize {
    // Prelude assignments are emitted as top-level statements of the shape
    // `globalThis.<name> ??= ...;`. This intentionally does not match
    // in-body reads like `globalThis.__phpxCurrentResponse.status`, only
    // the assignment lines themselves — that is what #47 made demand-driven.
    js.lines()
        .filter(|line| line.starts_with("globalThis.") && line.contains("??="))
        .count()
}

#[test]
fn emission_budget_line_count_ceiling() {
    let js = compile_budget_source();
    let line_count = js.lines().count();
    assert!(
        line_count <= MAX_EMITTED_LINES,
        "emitted line count grew from current (post-#47) reality: {line_count} lines \
         (ceiling {MAX_EMITTED_LINES}). If this growth is a new unconditional prelude write, \
         guard it with `want(\"...\")` in JsSubsetEmitter::finish instead of raising the ceiling. \
         Emitted JS:\n{js}"
    );
}

#[test]
fn emission_budget_globalthis_assignment_count_ceiling() {
    let js = compile_budget_source();
    let assignment_count = count_globalthis_prelude_assignments(&js);
    assert!(
        assignment_count <= MAX_GLOBALTHIS_ASSIGNMENTS,
        "globalThis.* prelude assignment count grew from current (post-#47) reality: \
         {assignment_count} (ceiling {MAX_GLOBALTHIS_ASSIGNMENTS}). BUDGET_SOURCE references none \
         of the demand-driven mission/runtime helpers, so this should stay at 0 — check for a new \
         unconditional `out.push_str(\"globalThis.Y ??= ...\")` in JsSubsetEmitter::finish that \
         isn't guarded by `want(\"Y\")`. Emitted JS:\n{js}"
    );
}

#[test]
fn emission_budget_program_code_is_present_despite_prelude() {
    // Sanity check that the gate is measuring the right artifact: the
    // program's own emitted code must still be present alongside whatever
    // prelude ships with it.
    let js = compile_budget_source();
    assert!(
        js.contains("function greet(name)"),
        "expected the program's own function in emitted output: {js}"
    );
    assert!(
        js.contains("export { greet }"),
        "expected the program's own function to be exported: {js}"
    );
    assert!(
        js.contains("hello \" + name"),
        "expected the program's own return expression in emitted output: {js}"
    );
}
