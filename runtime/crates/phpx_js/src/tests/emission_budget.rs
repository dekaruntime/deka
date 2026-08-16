// RFD 13 conformance gate #2 — emission budget.
//
// RFD 13 principle under test: "Impose as little runtime overhead as
// possible." Today, `compile_phpx_source_to_js` emits an unconditional
// `globalThis.*` prelude on every `.ds` program, whether or not the
// program references any of it (#47). For the 4-line `greet`/`print`
// program below, that prelude is currently ~49 unconditional
// `globalThis.X ??= ...` assignment lines out of ~82 total emitted lines —
// the program's own code is 8 of those 82 lines.
//
// Thresholds here are set at CURRENT REALITY PLUS A SMALL MARGIN, not at
// an aspirational target. They exist to catch the budget silently growing
// further, not to enforce #47's fix. As #47 lands (demand-driven prelude
// emission), these ceilings are meant to come DOWN — lowering them is the
// point, and a future PR that shrinks emitted output should shrink these
// numbers in the same commit as proof.

const BUDGET_SOURCE: &str = r#"export function greet(name: string): string {
    return "hello " + name;
}
print(greet("world"));
"#;

/// Ceiling on total emitted lines for `BUDGET_SOURCE`. Current reality: 82
/// lines. Margin: +8, to absorb incidental whitespace/formatting churn that
/// isn't itself a budget regression.
const MAX_EMITTED_LINES: usize = 90;

/// Ceiling on unconditional `globalThis.<name> ??= ...` prelude assignment
/// lines for `BUDGET_SOURCE`. Current reality: 49. Margin: +6.
const MAX_GLOBALTHIS_ASSIGNMENTS: usize = 55;

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
    // the assignment lines themselves — that is what #47 proposes making
    // demand-driven.
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
        "emitted line count grew from current reality: {line_count} lines (ceiling {MAX_EMITTED_LINES}). \
         If this is #47's demand-driven prelude landing, LOWER MAX_EMITTED_LINES in the same commit \
         instead of raising it. Emitted JS:\n{js}"
    );
}

#[test]
fn emission_budget_globalthis_assignment_count_ceiling() {
    let js = compile_budget_source();
    let assignment_count = count_globalthis_prelude_assignments(&js);
    assert!(
        assignment_count <= MAX_GLOBALTHIS_ASSIGNMENTS,
        "globalThis.* prelude assignment count grew from current reality: {assignment_count} \
         (ceiling {MAX_GLOBALTHIS_ASSIGNMENTS}). If this is #47's demand-driven prelude landing, \
         LOWER MAX_GLOBALTHIS_ASSIGNMENTS in the same commit instead of raising it. Emitted JS:\n{js}"
    );
}

#[test]
fn emission_budget_program_code_is_present_despite_prelude() {
    // Sanity check that the gate is measuring the right artifact: the
    // program's own emitted code must still be present alongside whatever
    // prelude ships with it.
    let js = compile_budget_source();
    assert!(
        js.contains("export function greet(name)"),
        "expected the program's own function in emitted output: {js}"
    );
    assert!(
        js.contains("hello \" + name"),
        "expected the program's own return expression in emitted output: {js}"
    );
}
