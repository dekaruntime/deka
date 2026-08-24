use super::*;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use std::path::Path;

fn ds_to_js(source: &str) -> Result<String, String> {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let msgs: Vec<&str> = program.errors.iter().map(|e| e.message).collect();
        return Err(format!("parse errors: {}", msgs.join("; ")));
    }
    let mut meta = SourceModuleMeta::empty();
    meta.is_ds = true;
    emit_js_from_ast(&program, source.as_bytes(), meta)
}

#[test]
fn ds_compiles_typed_bare_identifiers_dot_access_and_templates() {
    let source = "const prefix = `hello`; fn greet(user: Object) string { user.name; return `${prefix}`; }";
    let js = ds_to_js(source).expect("DekaScript should compile");
    assert!(js.contains("const prefix ="), "missing const: {js}");
    assert!(
        js.contains("function greet(user)"),
        "missing function: {js}"
    );
    assert!(js.contains("user.name"), "missing dot access: {js}");
}

#[test]
fn ds_preserves_top_level_let_before_const_initializer() {
    // Regression for deka#184: `const` declarations were emitted into the
    // frontmatter buffer, so they jumped ahead of preceding `let` statements.
    // That caused valid programs like `let a = 1; const b = a + 2` to throw a
    // TDZ error at runtime because `a` was referenced before initialization.
    let source = r#"
        let a = 1
        const b = a + 2
        console.log(a + b)
    "#;
    let js = ds_to_js(source).expect("DekaScript should compile");
    let let_pos = js
        .find("let a = 1")
        .expect("emitted JS should declare `let a = 1`");
    let const_pos = js
        .find("const b = deka.freeze(a + 2)")
        .expect("emitted JS should freeze `const b = a + 2`");
    assert!(
        let_pos < const_pos,
        "`let a` must appear before `const b` in emitted JS:\n{js}"
    );
}

#[test]
fn ds_compiler_entry_selects_native_mode_from_extension() {
    let source = "export const answer = 42;";
    let js = crate::compile_phpx_source_to_js(
        source,
        Path::new("lesson.ds").to_str().unwrap(),
        crate::parse_source_module_meta(source),
    )
    .expect(".ds source should compile through native mode");
    assert!(
        js.contains("const answer = deka.freeze(42)"),
        "missing emitted const: {js}"
    );
    assert!(
        js.contains("export { answer };"),
        "missing ESM export: {js}"
    );
}

#[test]
fn ds_lowers_native_string_collection_and_for_of_primitives() {
    let source = r#"
        fn joinWords(parts: Array<string>) string {
            let output = "";
            for (const part of parts) {
                output += part.slice(0, 1);
            }
            const first = parts[0];
            const meta = { first: first, count: parts.length };
            return `${first}:${output}`;
        }
        export { joinWords };
    "#;
    let js = crate::compile_phpx_source_to_js(
        source,
        "string/join_words.ds",
        crate::parse_source_module_meta(source),
    )
    .expect("native DekaScript string subset should compile");
    assert!(
        js.contains("let output = \"\""),
        "missing mutable binding: {js}"
    );
    assert!(
        js.contains("for (const part of"),
        "missing for-of lowering: {js}"
    );
    assert!(
        js.contains("output += part.slice(0, 1)"),
        "missing compound assignment: {js}"
    );
    assert!(js.contains("parts[0]"), "missing indexed list access: {js}");
    assert!(
        js.contains("const meta = deka.freeze({"),
        "missing frozen object literal: {js}"
    );
}

#[test]
fn ds_allows_declared_function_calls() {
    let source = r#"
        fn show<T>(value: T) void {
            print(value);
        }

        export { show };
        show(41);
    "#;
    let js = crate::compile_phpx_source_to_js(
        source,
        "show.ds",
        crate::parse_source_module_meta(source),
    )
    .expect("a declared DekaScript function should be callable");
    assert!(
        js.contains("function show(value)"),
        "missing function: {js}"
    );
    assert!(js.contains("show(41)"), "missing function call: {js}");
}

#[test]
fn ds_generic_variadic_identity_preserves_all_rest_values() {
    let source = r#"
        fn collect<T>(...values: Array<T>) Array<T> {
            return values;
        }
        export { collect };
    "#;
    let js = crate::compile_phpx_source_to_js(
        source,
        "array/collect.ds",
        crate::parse_source_module_meta(source),
    )
    .expect("Array<mixed> variadic identity should type-check");

    assert!(
        js.contains("function collect(...values)"),
        "variadic parameters must lower to a JS rest parameter so collect(1, 2, 3) retains every value: {js}"
    );
    assert!(
        js.contains("return values;"),
        "variadic identity must return the full rest array: {js}"
    );
}

#[test]
fn ds_rejects_nested_declarations() {
    // DekaScript declarations that the emitter hoists must not appear inside
    // conditional/block scopes, otherwise they would silently be lifted to
    // module scope (or, for receiver methods, silently dropped).
    for (source, expected) in [
        (
            r#"if (true) { fn helper() int { return 1 } }"#,
            "only allowed at the top level",
        ),
        (
            r#"if (true) { struct Point { x: int } }"#,
            "only allowed at the top level",
        ),
        (
            r#"if (true) { enum Color { Red } }"#,
            "only allowed at the top level",
        ),
        (
            r#"if (true) { fn (p Person) greet() string { return p.name } }"#,
            "only allowed at the top level",
        ),
    ] {
        let err = ds_to_js(source).expect_err("nested declaration must be rejected");
        assert!(
            err.contains(expected),
            "expected {expected:?} in {err:?} for source: {source}"
        );
    }
}

#[test]
fn ds_rejects_php_surface_and_const_reassignment() {
    for (source, expected) in [
        ("$name;", "bare identifiers"),
        ("echo 'hello';", "echo is not part"),
        ("array('hello');", "PHP array()"),
        ("(string) value;", "PHP casts"),
        ("foreach (items as item) {}", "foreach is not part"),
        ("left . right;", "concatenation is not part"),
        ("strlen(value);", "PHP built-ins are not part"),
        ("class Legacy {}", "PHP construct is not part"),
        ("namespace Legacy;", "PHP construct is not part"),
        ("global value;", "PHP construct is not part"),
        ("static value;", "PHP construct is not part"),
        ("try {} catch (error) {}", "PHP construct is not part"),
        ("throw value;", "PHP construct is not part"),
        ("include 'legacy.ds';", "include and require are not part"),
        (
            "require_once 'legacy.ds';",
            "include and require are not part",
        ),
    ] {
        let err = crate::compile_phpx_source_to_js(
            source,
            "invalid.ds",
            crate::parse_source_module_meta(source),
        )
        .expect_err("PHP surface must be rejected by .ds mode");
        assert!(err.contains(expected), "expected {expected:?} in {err:?}");
    }

    let err = ds_to_js("const answer = 42; answer += 1;")
        .expect_err("const reassignment must fail lowering");
    assert!(
        err.contains("immutable DekaScript const `answer`"),
        "unexpected error: {err}"
    );
}

#[test]
fn ds_lowering_errors_are_propagated_as_compile_failures() {
    for (source, expected) in [
        (
            "const x = 1; x++;",
            "cannot assign to immutable DekaScript const `x`",
        ),
        (
            "enum Color {\n  Red\n  Green\n  Blue\n}\nconst c = Color.Red\nconst result = match (c) {}\nconsole.log(result)",
            "match requires at least one arm",
        ),
        (
            "fn generator() {\n  yield 1\n}\nconsole.log(generator())",
            "yield expressions are not supported in JS subset emitter",
        ),
    ] {
        let err = crate::compile_phpx_source_to_js(
            source,
            "test.ds",
            crate::parse_source_module_meta(source),
        )
        .expect_err("DekaScript lowering error must be propagated");
        assert!(
            err.contains(expected),
            "expected {expected:?} in {err:?} for source: {source}"
        );
    }
}

#[test]
fn ds_c_style_for_loop_lowers_to_js_for() {
    let source = r#"
        let sum = 0
        for (let i = 0; i < 3; i = i + 1) {
            sum = sum + i
        }
        console.log(sum)
    "#;
    let js = ds_to_js(source).expect("C-style for loop should compile");
    assert!(
        js.contains("for (let i = 0; i < 3;"),
        "C-style for loop must lower to JS for: {js}"
    );
}

#[test]
fn ds_for_infinite_loop_with_break_lowers() {
    let source = r#"
        let n = 0
        for (;;) {
            n = n + 1
            if (n > 2) { break }
        }
        console.log(n)
    "#;
    let js = ds_to_js(source).expect("infinite for loop should compile");
    assert!(
        js.contains("for (; ; )"),
        "empty-clause for loop must lower to JS for: {js}"
    );
}

#[test]
fn ds_rejects_while_and_do_while() {
    for (source, expected) in [
        (
            "while (true) { break }",
            "`while` is not part of DekaScript",
        ),
        (
            "do { break } while (true)",
            "`do-while` is not part of DekaScript",
        ),
    ] {
        let err = ds_to_js(source).expect_err("while/do-while must be rejected in DS");
        assert!(
            err.contains(expected),
            "expected {expected:?} in {err:?} for source: {source}"
        );
    }
}

#[test]
fn ds_unsafe_block_returns_result_iife() {
    let js = ds_to_js("const answer = unsafe { JSON.parse(\"{\\\"x\\\":1}\") }")
        .expect("unsafe block should compile");
    assert!(
        js.contains("Ok(") && js.contains("JSON.parse") && js.contains("catch(err){return Err(err);}"),
        "unsafe should emit a Result-wrapping IIFE using bare prelude constructors:\n{js}"
    );
    assert!(
        js.contains("const Result = Object.freeze") && js.contains("const Option = Object.freeze"),
        "Result and Option prelude enums must be emitted:\n{js}"
    );
    assert!(
        js.contains("const Ok = Result.Ok") && js.contains("const Err = Result.Err"),
        "bare Result constructors must be aliased:\n{js}"
    );
    assert!(
        js.contains("__deka_host=void 0") && js.contains("__bridge=void 0"),
        "unsafe must hide the host dispatcher:\n{js}"
    );
    assert!(
        js.contains("(function(__g){")
            && js.contains("})(globalThis)")
            && !js.contains("}=unsafe;")
            && !js.contains("JSON.parse=(text)")
            && !js.contains("globalThis.fetch??="),
        "unsafe must use the host Proxy and must not wrap JSON/fetch on globalThis:\n{js}"
    );
}

#[test]
fn ds_unsafe_does_not_hijack_json_or_fetch() {
    let js = ds_to_js("const r = unsafe { JSON.parse(\"{}\") }\nconst f = unsafe { fetch(\"/x\") }\n")
        .expect("unsafe JSON/fetch should compile");
    assert!(
        !js.contains("globalThis.JSON.parse=")
            && !js.contains("__DekaJSONParse")
            && !js.contains("globalThis.fetch??="),
        "RFD 21: no ambient JSON/fetch wrappers:\n{js}"
    );
}

#[test]
fn ds_unsafe_await_emits_async_iife() {
    let js = ds_to_js("const r = unsafe { await fetch(url) }")
        .expect("unsafe await should compile");
    assert!(
        js.contains("async function") && js.contains("await fetch(url)"),
        "top-level await in unsafe must emit an async IIFE:\n{js}"
    );
}

#[test]
fn ds_unsafe_nested_async_fn_stays_sync_wrapper() {
    let js = ds_to_js(
        "const r = unsafe { async function load() { return await fetch(url) } return load() }",
    )
    .expect("nested async in unsafe should compile");
    assert!(
        js.contains("catch(err){return Err(err);}"),
        "expected Result IIFE:\n{js}"
    );
    // Wrapper is sync; inner function is async.
    assert!(
        js.contains("(function(){try{return Ok("),
        "wrapper without top-level await stays sync:\n{js}"
    );
}

#[test]
fn ds_host_import_emits_native_runtime() {
    let source = r#"
import { runtime, select } from "host"
const label = select({ native: "cli", browser: "wasm" })
console.log(runtime)
"#;
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    assert!(program.errors.is_empty(), "parse errors: {:?}", program.errors);
    let mut meta = parse_source_module_meta(source);
    meta.is_ds = true;
    let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("from host should compile");
    assert!(
        js.contains("HostRuntime.Native") && js.contains("function select("),
        "expected injected host module, got:\n{js}"
    );
    assert!(
        !js.contains("from 'host'"),
        "host must not emit a real import:\n{js}"
    );
}

#[test]
fn ds_bridge_crypto_digest_emits_host_call() {
    let js = ds_to_js("const x = bridge crypto.digest(\"sha256\", data)")
        .expect("bridge digest should emit");
    assert!(
        js.contains("Symbol.for(\"deka.host.internal\")")
            && js.contains(".host(\"crypto\", \"digest\""),
        "expected digest host call via internal symbol, got:\n{js}"
    );
}

#[test]
fn ds_bridge_fs_write_file_emits_async() {
    let js = ds_to_js("const x = await bridge fs.write_file(\"a.txt\", data)")
        .expect("bridge fs.write_file should emit");
    assert!(
        js.contains(".host(\"fs\", \"write_file\"") && js.contains("await "),
        "expected awaited fs write_file host call, got:\n{js}"
    );
}

#[test]
fn ds_bridge_net_connect_emits_host_call() {
    let js = ds_to_js("const x = bridge net.connect(\"127.0.0.1\", 80)")
        .expect("bridge net.connect should emit");
    assert!(
        js.contains("Symbol.for(\"deka.host.internal\")")
            && js.contains(".host(\"net\", \"connect\""),
        "expected net connect host call via internal symbol, got:\n{js}"
    );
}

#[test]
fn ds_bridge_tls_upgrade_emits_host_call() {
    let js = ds_to_js("const x = bridge tls.upgrade(1, \"localhost\")")
        .expect("bridge tls.upgrade should emit");
    assert!(
        js.contains("Symbol.for(\"deka.host.internal\")")
            && js.contains(".host(\"tls\", \"upgrade\""),
        "expected tls upgrade host call via internal symbol, got:\n{js}"
    );
}

#[test]
fn ds_bridge_async_fs_emits_await() {
    let js = ds_to_js("const x = await bridge fs.read_file(\"/tmp/a\")")
        .expect("async bridge should emit");
    assert!(
        js.contains("Symbol.for(\"deka.host.internal\")")
            && js.contains(".host(\"fs\", \"read_file\"")
            && js.contains("await "),
        "expected awaited fs host call via internal symbol, got:\n{js}"
    );
}

#[test]
fn ds_export_async_fn_stays_at_module_scope() {
    let source =
        "export async fn read_file(path: string) { return await bridge fs.read_file(path) }\n";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );
    let mut meta = parse_source_module_meta(source);
    meta.is_ds = true;
    let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("export async fn should emit");
    assert!(
        js.contains("export async function read_file"),
        "expected module-level export async function, got:\n{js}"
    );
    assert!(
        !js.contains("const __deka_main"),
        "export async fn must not be wrapped in __deka_main:\n{js}"
    );
}

#[test]
fn ds_unnamed_enum_payload_binds_in_match() {
    let source = r#"
        enum Msg { Text(string), Ping }
        fn body(m: Msg) string {
          return match (m) {
            Msg.Text(b) => b,
            Msg.Ping => "ok",
          }
        }
        console.log(body(Msg.Text("hi")))
    "#;
    let js = ds_to_js(source).expect("unnamed payload match should compile");
    assert!(
        js.contains("const b = ") && (js.contains(".__case === \"Text\"") || js.contains("__case === \"Text\"")),
        "payload pattern must bind the user name and discriminate on __case:\n{js}"
    );
}

#[test]
fn ds_named_enum_payload_and_match() {
    let source = r#"
        enum Outcome<T, E> {
            Win(value: T)
            Fail(error: E)
        }
        const r = Outcome.Win(42)
        const label = match (r) {
            Outcome.Win(value) => value,
            Outcome.Fail(error) => 0,
            _ => -1
        }
        console.log(label)
    "#;
    let js = ds_to_js(source).expect("named enum payload should compile");
    assert!(
        js.contains("Win: (value) => Object.freeze"),
        "named payload field must lower to a function with the given param name:\n{js}"
    );
    assert!(
        js.contains("r.__case === \"Win\"") || js.contains("__case === \"Win\""),
        "match guard must discriminate on __case:\n{js}"
    );
}

#[test]
fn ds_match_prelude_result_and_option_patterns() {
    let source = r#"
        const r = Result.Ok(42)
        const o = Option.Some("hi")
        console.log(match (r) { Ok(v) => v, Err(e) => -1 })
        console.log(match (o) { Some(v) => v, None => "empty" })
    "#;
    let js = ds_to_js(source).expect("prelude enum match patterns should compile");
    assert!(
        js.contains("r.__case === \"Ok\"") && js.contains("o.__case === \"Some\""),
        "match guards must discriminate on __case for prelude enums:\n{js}"
    );
    assert!(
        js.contains("const v = r[\"value\"]") || js.contains("const v = r['value']"),
        "Ok payload binding must read the value field:\n{js}"
    );
}

#[test]
fn ds_nested_match_patterns_emit_inner_tags() {
    let source = r#"
        fn f(r: Result<Option<number>, string>) number {
            return match (r) {
                Err(e) => 0,
                Ok(None) => 1,
                Ok(Some(v)) => v,
            }
        }
        console.log(f(Ok(Some(3))))
    "#;
    let js = ds_to_js(source).expect("nested match patterns should compile");
    assert!(
        js.contains("r.__case === \"Ok\"")
            && js.contains("__case === \"Some\"")
            && js.contains("__case === \"None\""),
        "nested match must discriminate inner Option tags:\n{js}"
    );
    assert!(
        js.contains("const v = r[\"value\"][\"value\"]")
            || js.contains("const v = r['value']['value']")
            || js.contains("r[\"value\"][\"value\"]"),
        "Ok(Some(v)) must bind through both payload fields:\n{js}"
    );
}

#[test]
fn ds_top_level_assignment_is_not_mirrored_to_globalthis() {
    let js = ds_to_js("x = 1").expect("top-level assignment should compile");
    assert!(
        js.contains("let x = 1"),
        "undeclared assignment should emit a module-local let:\n{js}"
    );
    assert!(
        !js.contains("globalThis.x"),
        "deka#228: top-level `x = 1` must not write globalThis.x:\n{js}"
    );
}

#[test]
fn ds_top_level_reassignment_is_not_mirrored_to_globalthis() {
    let js = ds_to_js("x = 1\nx = 2").expect("reassignment should compile");
    assert!(
        !js.contains("globalThis.x"),
        "deka#228: reassignment must not write globalThis.x:\n{js}"
    );
}

#[test]
fn ds_let_reassignment_is_not_mirrored_to_globalthis() {
    let js = ds_to_js("let x = 1\nx = 2").expect("let reassignment should compile");
    assert!(js.contains("let x = 1"), "missing let:\n{js}");
    assert!(
        !js.contains("globalThis.x"),
        "deka#228: `let x = 1; x = 2` must not write globalThis.x:\n{js}"
    );
}

#[test]
fn ds_explicit_globalthis_assignment_still_emits() {
    let js = ds_to_js("globalThis.x = 1").expect("explicit globalThis write should compile");
    assert!(
        js.contains("globalThis.x = 1"),
        "explicit globalThis.x = 1 must still emit:\n{js}"
    );
}

#[test]
fn ds_prelude_result_and_option_constructors() {
    let source = r#"
        const ok = Ok(42)
        const err = Err("bad")
        const some = Some(1)
        const none = None
        console.log(ok.__case)
        console.log(err.__case)
        console.log(some.__case)
        console.log(none.__case)
    "#;
    let js = ds_to_js(source).expect("prelude constructors should compile");
    assert!(
        js.contains("const Ok = Result.Ok") && js.contains("const Some = Option.Some"),
        "bare constructors must be aliased to prelude enum cases:\n{js}"
    );
}

#[test]
fn ds_unsafe_catch_is_rejected() {
    let err = ds_to_js("unsafe { 1 } catch (e) { 2 }")
        .expect_err("unsafe with catch should be rejected");
    assert!(
        !err.contains("unsafe { ... } catch { ... } is not supported") || err.contains("Unexpected") || err.contains("expected"),
        "unexpected error: {err}"
    );
}

#[test]
fn ds_enum_case_exposes_public_name() {
    let source = r#"
        enum Color { Red Green }
        console.log(Color.Red.name)
    "#;
    let js = ds_to_js(source).expect("enum name field should compile");
    assert!(
        js.contains("name: \"Red\"") && js.contains("__case: \"Red\""),
        "enum cases must carry a public name alongside the internal tag:\n{js}"
    );
}

#[test]
fn ds_panic_is_installed_as_a_language_global() {
    let js = ds_to_js(r#"panic("oops")"#).expect("panic() should compile");
    assert!(
        js.contains("globalThis.panic??=__deka.panic"),
        "bare panic() must bind the language global:\n{js}"
    );
}

#[test]
fn ds_js_ts_function_spellings_are_rejected() {
    let cases = [
        (
            r#"fn apply(f: fn(number) number, x: number) number { return f(x) }
console.log(apply((n) => n + 1, 5))"#,
            "Missing semicolon",
        ),
        (
            r#"fn apply(f: (n: number) => number, x: number) number { return f(x) }
fn inc(n: number) number { return n + 1 }
console.log(apply(inc, 5))"#,
            "DekaScript parameters use bare identifiers",
        ),
        (
            r#"function increment(n: number) number { return n + 1 }
console.log(increment(5))"#,
            "DekaScript uses `fn` for function declarations, not `function`",
        ),
        (
            r#"fn apply(f, x: number) number { return f(x) }
fn inc(n: number) number { return n + 1 }
console.log(apply(inc, 5))"#,
            "DekaScript parameters require a type annotation",
        ),
    ];
    for (source, needle) in cases {
        let err = ds_to_js(source).expect_err(source);
        assert!(
            err.contains(needle),
            "expected {needle:?} in {err:?} for {source}"
        );
    }

    for (source, needle) in [
        (
            r#"fn apply(f: Function, x: number) number { return f(x) }
fn inc(n: number) number { return n + 1 }
console.log(apply(inc, 5))"#,
            "Unknown type 'Function'",
        ),
        (
            r#"fn apply(f: any, x: number) number { return f(x) }
fn inc(n: number) number { return n + 1 }
console.log(apply(inc, 5))"#,
            "'any' and 'unknown' are not DekaScript types",
        ),
    ] {
        let err = crate::compile_phpx_source_to_js(
            source,
            "lesson.ds",
            crate::parse_source_module_meta(source),
        )
        .expect_err(source);
        assert!(
            err.contains(needle),
            "expected {needle:?} in {err:?} for {source}"
        );
    }
}

#[test]
fn ds_first_class_function_types_compile() {
    for source in [
        r#"fn applyTwice(f: fn(number) number, x: number) number { return f(f(x)) }
fn increment(n: number) number { return n + 1 }
console.log(applyTwice(increment, 5))"#,
        r#"type ProcessFunc = fn(string) string
fn shout(s: string) string { return s + "!" }
const f: ProcessFunc = shout
console.log(f("hi"))"#,
        r#"fn makeAdder(n: number) fn(number) number {
  return fn(x: number) number { return x + n }
}
const add10 = makeAdder(10)
console.log(add10(5))"#,
        r#"fn apply(f: fn(number) number, x: number) number { return f(x) }
console.log(apply(fn(n: number) number { return n + 7 }, 5))"#,
        r#"fn compose(f: fn(number) number, g: fn(number) number) fn(number) number {
  return fn(x: number) number { return f(g(x)) }
}
fn double(n: number) number { return n * 2 }
fn inc(n: number) number { return n + 1 }
console.log(compose(inc, double)(5))"#,
        r#"fn add1(n: number) number { return n + 1 }
fn mul2(n: number) number { return n * 2 }
const ops = [mul2, add1]
let n = 15
n = ops[0](n)
n = ops[1](n)
console.log(n)"#,
    ] {
        ds_to_js(source).unwrap_or_else(|err| panic!("expected compile, got {err} for {source}"));
    }
}

#[test]
fn ds_side_effect_import_emits_bare_import() {
    let source = "import \"./logger.ds\";\nconsole.log(1);\n";
    let mut meta = parse_source_module_meta(source);
    assert_eq!(meta.imports.len(), 1);
    assert!(meta.imports[0].specs.is_empty());
    assert_eq!(meta.imports[0].from, "./logger.ds");
    meta.is_ds = true;
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    assert!(program.errors.is_empty(), "parse errors: {:?}", program.errors);
    let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("side-effect import should emit");
    assert!(
        js.contains("import './logger.ds';"),
        "side-effect import must emit a bare import:\n{js}"
    );
}
