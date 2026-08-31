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
    run_dekascript_with_manifest(name, "{}\n", source, expected_output);
}

fn run_dekascript_with_manifest(name: &str, manifest: &str, source: &str, expected_output: &str) {
    run_dekascript_with_manifest_args(name, manifest, source, expected_output, &[]);
}

fn run_dekascript_with_manifest_args(
    name: &str,
    manifest: &str,
    source: &str,
    expected_output: &str,
    extra_args: &[&str],
) {
    let project = tempfile::tempdir().expect("create DekaScript project");
    fs::write(project.path().join("deka.json"), manifest).expect("write project manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("write project lockfile");
    let entry = project.path().join(format!("{name}.ds"));
    fs::write(&entry, source).expect("write DekaScript entry");

    let mut args = vec!["run"];
    args.extend_from_slice(extra_args);
    args.push(entry.to_str().expect("UTF-8 entry"));
    let output = Command::new(cli_bin())
        .args(&args)
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
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_console_log_without_deno() {
    run_dekascript(
        "console_log",
        "console.log(\"ok\")\n",
        "ok",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_unsafe_json_then_console_log() {
    run_dekascript(
        "unsafe_json_console",
        "const r = unsafe { JSON.parse('{\\\"x\\\":1}') }\nconsole.log(match (r) { Ok(v) => v.x, Err(e) => \"err\" })\n",
        "1",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_dekascript_comparison_candidate() {
    run_dekascript(
        "comparison",
        "const result = 7 > 3;\nprint(result);\n",
        "true",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_dekascript_boolean_candidate() {
    run_dekascript(
        "boolean",
        "const result = true && !false;\nprint(result);\n",
        "true",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_dekascript_if_else_candidate() {
    run_dekascript(
        "if_else",
        "const ready = false;\nif (ready) { print(\"wrong-branch\"); } else { print(\"if-else-ok\"); }\n",
        "if-else-ok",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_declared_array_function_call() {
    run_dekascript(
        "array_call",
        "export fn array(value: mixed) void { print(value); }\narray(41);\n",
        "41",
    );
}

// RFD 19: `self: Self` is how an impl-block method accesses its own
// receiver's fields (no implicit `this`, no PHP-style `$this`). This is
// the exact case that printed "hi from undefined" before self/this
// binding existed -- real end-to-end proof it now reads the real field.
#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_inherent_impl_method_reading_self_field() {
    run_dekascript(
        "inherent_impl_self",
        "struct Point { x: int; }\n\
         impl Point { doubled(self: Self): int { return self.x * 2; } }\n\
         const p = Point { x: 5 };\n\
         print(p.doubled());\n",
        "10",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_impl_method_calling_sibling_method_via_self() {
    // Regression test for a real bug found and fixed live: self.method()
    // calls inside an impl method body were misdiagnosed as unknown field
    // accesses by the first cut of the self.field validator. Verifies the
    // fix end-to-end, not just typechecked.
    run_dekascript(
        "impl_self_method_call",
        "struct Point { x: int; }\n\
         impl Point {\n\
           double(self: Self): int { return self.x * 2; }\n\
           quad(self: Self): int { return self.double() * 2; }\n\
         }\n\
         const p = Point { x: 3 };\n\
         print(p.quad());\n",
        "12",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_trait_impl_method_reading_self_field() {
    run_dekascript(
        "trait_impl_self",
        "trait Greeter {\n  greet(self: Self): string\n}\n\
         struct Bot { name: string; }\n\
         impl Greeter for Bot { greet(self: Self): string { return \"hi from \" + self.name; } }\n\
         const b = Bot { name: \"Rex\" };\n\
         print(b.greet());\n",
        "hi from Rex",
    );
}

// deka#71: impl Trait for Enum typechecked clean but silently produced no
// runtime method at all -- `c.label is not a function`. Fixed by moving
// method registration into emit_program's pre-pass so it runs before
// Stmt::Enum's own emission regardless of source order.
#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
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

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_trait_default_method_when_not_overridden() {
    // Real bug found and fixed live: a trait impl that doesn't override
    // one of the trait's default methods typechecked clean (the
    // typechecker correctly allows a non-overridden default to satisfy
    // conformance) but crashed at runtime with "x.method is not a
    // function" -- codegen was only ever emitting what the impl block's
    // OWN members provided, never falling back to the trait's default
    // body for methods left unoverridden.
    run_dekascript(
        "trait_default_not_overridden",
        "trait Shape {\n  area(self: Self): int\n  describe(self: Self): string { return \"a shape\"; }\n}\n\
         struct Square { side: int; }\n\
         impl Shape for Square { area(self: Self): int { return self.side * self.side; } }\n\
         const s = Square { side: 4 };\n\
         print(s.describe());\n",
        "a shape",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_impl_override_wins_over_trait_default() {
    // Same fix, opposite direction: when the impl DOES override a default,
    // the override must win, not silently get shadowed by the merge logic
    // that adds trait defaults for methods "not already provided."
    run_dekascript(
        "trait_default_overridden",
        "trait Shape {\n  describe(self: Self): string { return \"a shape\"; }\n}\n\
         struct Square { side: int; }\n\
         impl Shape for Square { describe(self: Self): string { return \"a square override\"; } }\n\
         const s = Square { side: 4 };\n\
         print(s.describe());\n",
        "a square override",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_trait_default_method_when_trait_declared_after_its_impl() {
    // Order-independence for the default-method fix, mirroring deka#71's
    // enum-impl fix: the trait declaring the default appears AFTER the
    // impl block that relies on it.
    run_dekascript(
        "trait_default_declared_after_impl",
        "struct Square { side: int; }\n\
         impl Shape for Square { area(self: Self): int { return self.side * self.side; } }\n\
         trait Shape {\n  area(self: Self): int\n  describe(self: Self): string { return \"declared after its impl\"; }\n}\n\
         const s = Square { side: 3 };\n\
         print(s.describe());\n",
        "declared after its impl",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_two_traits_each_contributing_a_default_method() {
    // A struct implementing two DIFFERENT traits via two separate impl
    // blocks, neither overriding its trait's default -- confirms the
    // per-impl-block merge (struct_methods.entry(...).extend(...)) doesn't
    // clobber defaults contributed by a sibling impl block for the same
    // target.
    run_dekascript(
        "two_trait_defaults_merge",
        "trait Reader { readLabel(self: Self): string { return \"reading\"; } }\n\
         trait Writer { writeLabel(self: Self): string { return \"writing\"; } }\n\
         struct Conn { id: int; }\n\
         impl Reader for Conn { }\n\
         impl Writer for Conn { }\n\
         const c = Conn { id: 1 };\n\
         print(c.readLabel() + \" \" + c.writeLabel());\n",
        "reading writing",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_trait_default_method_for_enum_target_when_not_overridden() {
    // Same default-method fix, verified on an ENUM target -- struct_methods
    // is a shared map keyed by target name regardless of struct vs enum, so
    // this was expected to already work without separate handling, but
    // hadn't been directly verified until now.
    run_dekascript(
        "trait_default_enum_target",
        "trait Namer {\n  label(self: Self): string\n  describe(self: Self): string { return \"an enum value\"; }\n}\n\
         enum Color { case Red; case Green; }\n\
         impl Namer for Color { label(self: Self): string { return \"a color\"; } }\n\
         const c = Color::Red;\n\
         print(c.label() + \" \" + c.describe());\n",
        "a color an enum value",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_template_method_pattern_default_calling_abstract() {
    // The most common real-world trait idiom: a default method that calls
    // an abstract method the impl is required to provide (the "template
    // method" pattern). Exercises both fixes from tonight together -- the
    // self.method() call fix (self.area() inside the DEFAULT body, not an
    // impl-provided one) and the default-method emission fix (describe()
    // itself must be attached even though Square never overrides it).
    // JS method dispatch on `this.area()` resolves correctly because both
    // the trait default and the impl's own methods end up merged onto the
    // same struct_methods entry for Square.
    run_dekascript(
        "template_method_pattern",
        "trait Shape {\n  area(self: Self): int\n  describe(self: Self): string { return \"area is \" + self.area(); }\n}\n\
         struct Square { side: int; }\n\
         impl Shape for Square { area(self: Self): int { return self.side * self.side; } }\n\
         const s = Square { side: 5 };\n\
         print(s.describe());\n",
        "area is 25",
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
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
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
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_impl_before_struct_declaration_method() {
    run_dekascript(
        "impl_before_struct",
        "impl Point { norm(self: Self): int { return self.x * 2; } }\n\
         struct Point { x: int; }\n\
         const p = Point { x: 5 };\n\
         print(p.norm());\n",
        "10",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
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
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_dekascript_generic_variadic_collect_candidate() {
    run_dekascript(
        "collect",
        "export fn collect(...values: Array<mixed>) Array<mixed> {\n    return values;\n}\nconst result = collect(1, 2, 3);\nprint(result);\n",
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
        // Not the whole sentence: it became ".ds/.dsx entrypoints" when RFD 24
        // phase 2 added the extension, and the exact wording is not what this
        // test is about. The load-bearing assertions are the two around it --
        // non-zero exit, and the entry never executing.
        combined.contains("Run mode supports .ds"),
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
        combined.contains("Run mode supports .ds"),
        "missing PHPX target rejection in runtime output: {combined}"
    );
    assert!(
        !combined.contains("must-not-execute"),
        "PHPX symlink target reached execution: {combined}"
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_workspace_fs_write_read_dir() {
    run_dekascript_with_manifest(
        "fs_write_read",
        r#"{ "name": "@deka/fs", "host": { "kinds": ["fs"] }, "security": { "allow": { "read": ["./"], "write": ["./"] }, "prompt": false } }"#,
        r#"
export async fn read_file(path: string) {
  return await bridge fs.read_file(path)
}
export async fn write_file(path: string, data: bytes) {
  return await bridge fs.write_file(path, data)
}
export async fn mkdirs(path: string) {
  return await bridge fs.mkdirs(path)
}
export async fn read_dir(path: string) {
  return await bridge fs.read_dir(path)
}
fn utf8(value: string) bytes {
  let encoded = unsafe { new TextEncoder().encode(value) }
  return match (encoded) {
    Ok(buf) => buf,
    Err(err) => utf8("")
  }
}
fn from_utf8(value: bytes) string {
  let decoded = unsafe { new TextDecoder("utf-8", { fatal: false }).decode(value) }
  return match (decoded) {
    Ok(text) => text,
    Err(err) => ""
  }
}
async fn go() {
  const mkdir_res = await mkdirs("out")
  const made = match (mkdir_res) {
    Ok(v) => true,
    Err(e) => false
  }
  if (!made) {
    print("fail-mkdirs")
    return
  }
  const write_res = await write_file("out/a.txt", utf8("hi"))
  const wrote = match (write_res) {
    Ok(n) => n,
    Err(e) => 0
  }
  const read_res = await read_file("out/a.txt")
  const text = match (read_res) {
    Ok(buf) => from_utf8(buf),
    Err(e) => "fail-read"
  }
  const dir_res = await read_dir("out")
  const names = match (dir_res) {
    Ok(entries) => match (unsafe { entries.map((e) => e.name).join(",") }) {
      Ok(v) => v,
      Err(e) => "fail-dir"
    },
    Err(e) => "fail-dir"
  }
  const has = match (unsafe { String(names).indexOf("a.txt") >= 0 }) {
    Ok(v) => v,
    Err(e) => false
  }
  if (wrote > 0 && text == "hi" && has) {
    print("ok")
  } else {
    print(text)
    print(names)
  }
}
go()
"#,
        "ok",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_workspace_time_sleep_ms() {
    run_dekascript_with_manifest(
        "time_sleep",
        r#"{ "name": "@deka/time", "host": { "kinds": ["time"] } }"#,
        r#"
export fn sleep_ms(ms: number) {
  return bridge time.sleep_ms(ms)
}
fn go() {
  print(match (sleep_ms(1)) {
    Ok(v) => "ok",
    Err(e) => "fail"
  })
}
go()
"#,
        "ok",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_workspace_tcp_connect_refused() {
    run_dekascript_with_manifest(
        "tcp_connect_refused",
        r#"{ "name": "@deka/tcp", "host": { "kinds": ["net"] }, "security": { "allow": { "net": ["127.0.0.1:1"] }, "prompt": false } }"#,
        r#"
export fn connect(host: string, port: number) {
  return bridge net.connect(host, port)
}
fn go() {
  print(match (connect("127.0.0.1", 1)) {
    Ok(h) => "fail-ok",
    Err(e) => "ok"
  })
}
go()
"#,
        "ok",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_workspace_tls_upgrade_unknown_handle() {
    run_dekascript_with_manifest_args(
        "tls_upgrade_unknown",
        r#"{ "name": "@deka/tls", "host": { "kinds": ["tls"] } }"#,
        r#"
export fn upgrade(handle: number, server_name: string) {
  return bridge tls.upgrade(handle, server_name)
}
fn go() {
  print(match (upgrade(0, "localhost")) {
    Ok(h) => "fail-ok",
    Err(e) => "ok"
  })
}
go()
"#,
        "ok",
        &["--no-prompt"],
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_workspace_crypto_bridge() {
    run_dekascript_with_manifest(
        "crypto_bridge",
        r#"{ "name": "@deka/crypto", "host": { "kinds": ["crypto"] } }"#,
        r#"
export fn random_bytes(len: number) {
  return bridge crypto.random_bytes(len)
}
export fn digest(algorithm: string, data: bytes) {
  return bridge crypto.digest(algorithm, data)
}
export fn hmac(algorithm: string, key: bytes, data: bytes) {
  return bridge crypto.hmac(algorithm, key, data)
}
export fn secure_compare(a: bytes, b: bytes) {
  return bridge crypto.secure_compare(a, b)
}
fn go() {
  const r = random_bytes(16)
  print(match (r) {
    Ok(v) => match (digest("sha256", v)) {
      Ok(h) => match (hmac("sha256", v, h)) {
        Ok(mac) => match (secure_compare(mac, mac)) {
          Ok(eq) => "ok",
          Err(e) => "fail-compare"
        },
        Err(e) => "fail-hmac"
      },
      Err(e) => "fail-digest"
    },
    Err(e) => "fail-random"
  })
}
go()
"#,
        "ok",
    );
}

#[test]
#[ignore = "v1-only syntax (impl/trait/bridge/print/mixed/console); revisit after v2 feature parity (see dekaruntime/deka#330)"]
fn run_executes_jwt_hs256_on_crypto_host_ops() {
    run_dekascript_with_manifest(
        "jwt_hs256",
        r#"{ "name": "@deka/crypto", "host": { "kinds": ["crypto"] } }"#,
        r#"
export fn hmac(algorithm: string, key: bytes, data: bytes) {
  return bridge crypto.hmac(algorithm, key, data)
}
export fn secure_compare(a: bytes, b: bytes) {
  return bridge crypto.secure_compare(a, b)
}

fn utf8(value: string) bytes {
  let encoded = unsafe { new TextEncoder().encode(value) }
  return match (encoded) {
    Ok(buf) => buf,
    Err(err) => utf8("")
  }
}

fn from_utf8(value: bytes) string {
  let decoded = unsafe { new TextDecoder("utf-8", { fatal: false }).decode(value) }
  return match (decoded) {
    Ok(text) => text,
    Err(err) => ""
  }
}

fn b64url_encode(data: bytes) string {
  let r = unsafe {
    var alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    var s = "";
    var i = 0;
    var n = data.byteLength;
    for (i = 0; i < n; i += 3) {
      var b0 = data[i];
      var b1 = i + 1 < n ? data[i + 1] : 0;
      var b2 = i + 2 < n ? data[i + 2] : 0;
      var triple = (b0 << 16) + (b1 << 8) + b2;
      s += alphabet[(triple >> 18) & 63];
      s += alphabet[(triple >> 12) & 63];
      s += i + 1 < n ? alphabet[(triple >> 6) & 63] : "=";
      s += i + 2 < n ? alphabet[triple & 63] : "=";
    }
    return s.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
  }
  return match (r) {
    Ok(v) => v,
    Err(e) => ""
  }
}

fn b64url_decode(value: string) bytes {
  let r = unsafe {
    var alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    var s = String(value).replace(/-/g, "+").replace(/_/g, "/");
    while (s.length % 4 !== 0) s += "=";
    var clean = s.replace(/=+$/, "");
    var n = clean.length;
    var out = [];
    var i = 0;
    for (i = 0; i < n; i += 4) {
      var c0 = alphabet.indexOf(clean[i]);
      var c1 = alphabet.indexOf(clean[i + 1]);
      var c2 = i + 2 < n ? alphabet.indexOf(clean[i + 2]) : 0;
      var c3 = i + 3 < n ? alphabet.indexOf(clean[i + 3]) : 0;
      var triple = (c0 << 18) + (c1 << 12) + (c2 << 6) + c3;
      out.push((triple >> 16) & 255);
      if (i + 2 < n) out.push((triple >> 8) & 255);
      if (i + 3 < n) out.push(triple & 255);
    }
    return new Uint8Array(out);
  }
  return match (r) {
    Ok(buf) => buf,
    Err(err) => utf8("")
  }
}

export fn sign_hs256(payload_json: string, secret: bytes) {
  const header_json = '{"alg":"HS256","typ":"JWT"}'
  const input = b64url_encode(utf8(header_json)) + "." + b64url_encode(utf8(payload_json))
  return match (hmac("sha256", secret, utf8(input))) {
    Ok(sig) => Ok(input + "." + b64url_encode(sig)),
    Err(e) => Err(e)
  }
}

fn jwt_parts(token: string) {
  return unsafe {
    var p = String(token).split(".");
    if (p.length !== 3) throw new Error("invalid jwt format");
    return { header: p[0], payload: p[1], signature: p[2], input: p[0] + "." + p[1] };
  }
}

fn header_alg(header_b64: string) {
  const json = from_utf8(b64url_decode(header_b64))
  return unsafe { String(JSON.parse(json).alg || "") }
}

fn require_hs256(alg: string) {
  if (alg == "HS256") {
    return Ok(true)
  }
  return Err("unsupported jwt alg")
}

fn require_signature(eq: boolean) {
  if (eq) {
    return Ok(true)
  }
  return Err("invalid jwt signature")
}

export fn verify_hs256(token: string, secret: bytes) {
  return match (jwt_parts(token)) {
    Err(e) => Err("invalid jwt format"),
    Ok(p) => match (header_alg(p.header)) {
      Err(e) => Err("invalid jwt header"),
      Ok(alg) => match (require_hs256(alg)) {
        Err(e) => Err(e),
        Ok(_) => match (hmac("sha256", secret, utf8(p.input))) {
          Err(e) => Err(e),
          Ok(expected) => match (secure_compare(expected, b64url_decode(p.signature))) {
            Err(e) => Err(e),
            Ok(eq) => match (require_signature(eq)) {
              Err(e) => Err(e),
              Ok(_) => Ok(from_utf8(b64url_decode(p.payload)))
            }
          }
        }
      }
    }
  }
}

fn go() {
  let secret = utf8("secret")
  const payload = '{"sub":"1234567890","name":"John Doe","iat":1516239022}'
  const expected = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.XbPfbIHMI6arZ3Y922BhjWgQzWXcXNrz0ogtVhfEd2o"
  const signed = match (sign_hs256(payload, secret)) {
    Ok(v) => v,
    Err(e) => "fail-sign"
  }
  const verified = match (verify_hs256(signed, secret)) {
    Ok(v) => v,
    Err(e) => "fail-verify"
  }
  const none_header = b64url_encode(utf8('{"alg":"none"}'))
  const none_payload = b64url_encode(utf8('{"sub":"1"}'))
  const none_tok = none_header + "." + none_payload + "."
  const none_rejected = match (verify_hs256(none_tok, secret)) {
    Ok(v) => false,
    Err(e) => true
  }
  const tampered = match (unsafe { signed.slice(0, -1) + (signed.slice(-1) === "o" ? "x" : "o") }) {
    Ok(v) => v,
    Err(e) => signed
  }
  const tamper_rejected = match (verify_hs256(tampered, secret)) {
    Ok(v) => false,
    Err(e) => true
  }
  if (signed != expected) {
    print("fail-vector")
  } else if (verified != payload) {
    print("fail-verify")
  } else if (!none_rejected) {
    print("fail-none")
  } else if (!tamper_rejected) {
    print("fail-tamper")
  } else {
    print("ok")
  }
}
go()
"#,
        "ok",
    );
}

/// deka#394: a bare `None` literal emitted JS `null` while `match` tested
/// `__case === "None"`, so matching on any function that returned `None` threw
/// `TypeError: Cannot read properties of null`. This executes the program, so
/// it fails on the throw rather than on emitted text.
#[test]
fn bare_none_matches_as_option_none() {
    run_dekascript(
        "bare_none_matches",
        r#"
fn find(hit: boolean): Option<number> {
  if (hit) {
    return Some(7)
  }
  return None
}

fn show(hit: boolean): number {
  return match (find(hit)) {
    Some(v) => v,
    None => 0
  }
}

const hit = show(true);
const miss = show(false);
unsafe { console.log(hit) };
unsafe { console.log(miss) };
"#,
        "7\n0",
    );
}

/// deka#394: a `None` literal bound directly must be the same value as one
/// returned from a function, so both match the same arm.
#[test]
fn bare_none_binding_matches_prelude_none() {
    run_dekascript(
        "bare_none_binding",
        r#"
fn label(value: Option<number>): string {
  return match (value) {
    Some(v) => "some",
    None => "none"
  }
}

const direct: Option<number> = None;
const wrapped: Option<number> = Some(3);
const a = label(direct);
const b = label(wrapped);
unsafe { console.log(a) };
unsafe { console.log(b) };
"#,
        "none\nsome",
    );
}
