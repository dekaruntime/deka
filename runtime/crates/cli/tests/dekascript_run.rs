use std::fs;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Minimal valid lockfile content. `ensure_project_layout` (crates/pool/src/esm_loader.rs)
/// only checks that `deka.lock` exists at the project root — it does not require any
/// specific packages — but a project root with a `deka.json` and no `deka.lock` is
/// rejected before the program ever executes ("deka runtime requires deka.lock at
/// project root"). Mirrors the lockfile shape used by
/// crates/cli/tests/update_integrity_process.rs and the `deka init` output.
const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;

fn run_dekascript(name: &str, source: &str, expected_output: &str) {
    let project = tempfile::tempdir().expect("create DekaScript project");
    fs::write(project.path().join("deka.json"), "{}\n").expect("write project manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("write project lockfile");
    let entry = project.path().join(format!("{name}.ds"));
    fs::write(&entry, source).expect("write DekaScript entry");

    let output = Command::new(cli_bin())
        .args(["run", entry.to_str().expect("UTF-8 entry")])
        .current_dir(project.path())
        .output()
        .expect("run DekaScript entry through the CLI runtime");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "DekaScript run failed: {combined}");
    assert!(
        combined.contains(expected_output),
        "missing {expected_output:?} in runtime output: {combined}"
    );
}

#[test]
fn run_executes_dekascript_comparison_candidate() {
    run_dekascript(
        "comparison",
        "const result = 7 > 3;\nprint(result);\n",
        "true",
    );
}

#[test]
fn run_executes_dekascript_boolean_candidate() {
    run_dekascript(
        "boolean",
        "const result = true && !false;\nprint(result);\n",
        "true",
    );
}

#[test]
fn run_executes_dekascript_if_else_candidate() {
    run_dekascript(
        "if_else",
        "const ready = false;\nif (ready) { print(\"wrong-branch\"); } else { print(\"if-else-ok\"); }\n",
        "if-else-ok",
    );
}

#[test]
fn run_executes_declared_array_function_call() {
    run_dekascript(
        "array_call",
        "export function array(value: mixed): void { print(value); }\narray(41);\n",
        "41",
    );
}

// RFD 19: `self: Self` is how an impl-block method accesses its own
// receiver's fields (no implicit `this`, no PHP-style `$this`). This is
// the exact case that printed "hi from undefined" before self/this
// binding existed -- real end-to-end proof it now reads the real field.
#[test]
fn run_executes_inherent_impl_method_reading_self_field() {
    run_dekascript(
        "inherent_impl_self",
        "struct Point { $x: int; }\n\
         impl Point { doubled(self: Self): int { return self.x * 2; } }\n\
         const p = Point { $x: 5 };\n\
         print(p.doubled());\n",
        "10",
    );
}

#[test]
fn run_executes_impl_method_calling_sibling_method_via_self() {
    // Regression test for a real bug found and fixed live: self.method()
    // calls inside an impl method body were misdiagnosed as unknown field
    // accesses by the first cut of the self.field validator. Verifies the
    // fix end-to-end, not just typechecked.
    run_dekascript(
        "impl_self_method_call",
        "struct Point { $x: int; }\n\
         impl Point {\n\
           double(self: Self): int { return self.x * 2; }\n\
           quad(self: Self): int { return self.double() * 2; }\n\
         }\n\
         const p = Point { $x: 3 };\n\
         print(p.quad());\n",
        "12",
    );
}

#[test]
fn run_executes_trait_impl_method_reading_self_field() {
    run_dekascript(
        "trait_impl_self",
        "trait Greeter {\n  greet(self: Self): string\n}\n\
         struct Bot { $name: string; }\n\
         impl Greeter for Bot { greet(self: Self): string { return \"hi from \" + self.name; } }\n\
         const b = Bot { $name: \"Rex\" };\n\
         print(b.greet());\n",
        "hi from Rex",
    );
}

// deka#71: impl Trait for Enum typechecked clean but silently produced no
// runtime method at all -- `c.label is not a function`. Fixed by moving
// method registration into emit_program's pre-pass so it runs before
// Stmt::Enum's own emission regardless of source order.
#[test]
fn run_executes_impl_trait_for_enum_method() {
    run_dekascript(
        "impl_for_enum",
        "trait Namer {\n  label(self: Self): string\n}\n\
         enum Color { case Red; case Green; }\n\
         impl Namer for Color { label(self: Self): string { return \"a color\"; } }\n\
         const c = Color::Red;\n\
         print(c.label());\n",
        "a color",
    );
}

// Same case, but with `impl` appearing BEFORE the `enum` it targets --
// proves the fix is genuinely order-independent, not incidentally correct
// for one source ordering.
// Inherent impl (no trait) for an enum -- a distinct combination from the
// trait-impl-for-enum cases above, confirming the fix in deka#71 was
// correctly unconditional on trait_name rather than only fixing the
// trait-impl path.
#[test]
fn run_executes_inherent_impl_for_enum_method() {
    run_dekascript(
        "inherent_impl_enum",
        "enum Color { case Red; case Green; }\n\
         impl Color { describe(self: Self): string { return \"a color value\"; } }\n\
         const c = Color::Green;\n\
         print(c.describe());\n",
        "a color value",
    );
}

// Struct target, impl declared BEFORE the struct -- rounds out the
// ordering matrix alongside the enum cases above. Structs read their
// method table at construction time (always later than both declarations,
// regardless of source order), so this was expected to already work --
// verified rather than assumed.
#[test]
fn run_executes_impl_before_struct_declaration_method() {
    run_dekascript(
        "impl_before_struct",
        "impl Point { norm(self: Self): int { return self.x * 2; } }\n\
         struct Point { $x: int; }\n\
         const p = Point { $x: 5 };\n\
         print(p.norm());\n",
        "10",
    );
}

#[test]
fn run_executes_impl_before_enum_declaration_method() {
    run_dekascript(
        "impl_before_enum",
        "trait Namer {\n  label(self: Self): string\n}\n\
         impl Namer for Color { label(self: Self): string { return \"reversed order works\"; } }\n\
         enum Color { case Red; case Green; }\n\
         const c = Color::Red;\n\
         print(c.label());\n",
        "reversed order works",
    );
}

#[test]
fn run_executes_dekascript_generic_variadic_collect_candidate() {
    run_dekascript(
        "collect",
        "export function collect(...values: Array<mixed>): Array<mixed> {\n    return values;\n}\nconst result = collect(1, 2, 3);\nprint(result);\n",
        "1,2,3",
    );
}

#[test]
fn run_rejects_phpx_entry_before_execution() {
    let project = tempfile::tempdir().expect("create PHPX project");
    fs::write(project.path().join("deka.json"), "{}\n").expect("write project manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("write project lockfile");
    let entry = project.path().join("legacy.phpx");
    fs::write(&entry, "print(\"must-not-execute\");\n").expect("write PHPX entry");

    let output = Command::new(cli_bin())
        .args(["run", entry.to_str().expect("UTF-8 entry")])
        .current_dir(project.path())
        .output()
        .expect("run PHPX entry through the CLI runtime");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !output.status.success(),
        "PHPX entry unexpectedly ran: {combined}"
    );
    assert!(
        combined.contains("Run mode supports .ds entrypoints"),
        "missing PHPX rejection in runtime output: {combined}"
    );
    assert!(
        !combined.contains("must-not-execute"),
        "PHPX entry reached execution: {combined}"
    );
}

#[cfg(unix)]
#[test]
fn run_rejects_absolute_ds_symlink_to_phpx_before_execution() {
    let project = tempfile::tempdir().expect("create DekaScript project");
    fs::write(project.path().join("deka.json"), "{}\n").expect("write project manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("write project lockfile");
    let target = project.path().join("legacy.phpx");
    fs::write(&target, "print(\"must-not-execute\");\n").expect("write PHPX target");
    let entry = project.path().join("entry.ds");
    std::os::unix::fs::symlink(&target, &entry).expect("create DekaScript symlink");

    let output = Command::new(cli_bin())
        .args(["run", entry.to_str().expect("UTF-8 entry")])
        .current_dir(project.path())
        .output()
        .expect("run absolute DekaScript symlink through the CLI runtime");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !output.status.success(),
        "PHPX symlink target unexpectedly ran: {combined}"
    );
    assert!(
        combined.contains("Run mode supports .ds entrypoints"),
        "missing PHPX target rejection in runtime output: {combined}"
    );
    assert!(
        !combined.contains("must-not-execute"),
        "PHPX symlink target reached execution: {combined}"
    );
}
