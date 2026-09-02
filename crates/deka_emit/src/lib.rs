//! DekaScript JavaScript emitter (Compiler v2).
//!
//! Emits reasonably formatted JavaScript from the v2 AST, erasing type
//! annotations. Structs become `deka.Struct` factories, enums become frozen
//! case objects, and receiver methods are registered on the factory prototype
//! so instance method calls work without a separate lowering pass.

mod emit;
mod util;

pub use emit::{emit_js, emit_js_with_imports, emit_js_with_options};

#[cfg(test)]
mod tests {
    use super::*;
    use bumpalo::Bump;
    use deka_syntax::parse;

    fn parse_and_emit(source: &str) -> String {
        let arena = Bump::new();
        let result = parse(source, &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.expect("parse produced no program");
        emit_js(&program, source).expect("emit failed")
    }

    /// Union type-patterns are lowered by the typechecker, so emission of
    /// them needs the checker results — unlike the erase-only `parse_and_emit`.
    fn parse_check_and_emit(source: &str) -> String {
        let arena = Bump::new();
        let result = parse(source, &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.expect("parse produced no program");
        let typeck = deka_syntax::typeck::check_program(&program, source);
        assert!(typeck.errors.is_empty(), "{:?}", typeck.errors);
        emit_js_with_options(
            &program,
            source,
            &std::collections::HashMap::new(),
            None,
            &typeck.unwrap_calls,
            &typeck.operator_rewrites,
            &typeck.jsx_optional_props,
            &typeck.enum_case_patterns,
            &typeck.union_type_patterns,
            "module.ds",
            None,
        )
        .expect("emit failed")
    }

    #[test]
    fn emit_union_type_pattern_primitive_predicates() {
        let out = parse_check_and_emit(
            "fn f(v: string | number) string { return match (v) { string(s) => s, number(n) => string(n) }; }",
        );
        assert!(
            out.contains("typeof __deka_scrutinee === \"string\""),
            "expected typeof predicate, got: {}",
            out
        );
        assert!(
            out.contains("typeof __deka_scrutinee === \"number\""),
            "expected typeof predicate, got: {}",
            out
        );
        // The payload is bound to the scrutinee itself.
        assert!(out.contains("const s = __deka_scrutinee;"), "got: {}", out);
        assert!(out.contains("const n = __deka_scrutinee;"), "got: {}", out);
        // Primitives have no __case tag; emitting one would mean the union
        // lookup was skipped.
        assert!(!out.contains("__case === \"string\""), "got: {}", out);
    }

    #[test]
    fn emit_union_type_pattern_struct_predicate() {
        let out = parse_check_and_emit(
            "struct Point { x: number; y: number }\nfn f(v: Point | string) number { return match (v) { Point(p) => p.x, string(s) => s.length }; }",
        );
        assert!(
            out.contains("deka.getStructId(__deka_scrutinee) === \"Point\""),
            "expected getStructId predicate, got: {}",
            out
        );
        // The struct prelude (which defines getStructId) must be emitted.
        assert!(out.contains("getStructId:"), "got: {}", out);
        assert!(out.contains("const p = __deka_scrutinee;"), "got: {}", out);
    }

    #[test]
    fn emit_union_type_pattern_boolean_and_bytes_predicates() {
        let out = parse_check_and_emit(
            "fn f(v: boolean | bytes) number { return match (v) { boolean(b) => 1, bytes(raw) => 2 }; }",
        );
        assert!(
            out.contains("typeof __deka_scrutinee === \"boolean\""),
            "got: {}",
            out
        );
        assert!(
            out.contains("__deka_scrutinee instanceof Uint8Array"),
            "got: {}",
            out
        );
    }

    #[test]
    fn emit_const_number() {
        let out = parse_and_emit("const x = 42;");
        assert!(out.contains("const x = 42;"), "got: {}", out);
    }

    #[test]
    fn emit_function_with_return() {
        let out = parse_and_emit("fn add(a: number, b: number) number { return a + b; }");
        assert!(out.contains("function add(a, b) {"), "got: {}", out);
        assert!(out.contains("return a + b;"), "got: {}", out);
    }

    #[test]
    fn emit_call_expression() {
        let out = parse_and_emit("console.log(\"hello\");");
        assert!(out.contains("console.log(\"hello\");"), "got: {}", out);
    }

    #[test]
    fn emit_match_expression() {
        let out = parse_and_emit(
            "const o = Some(5); const x = match o { Some(n) => n, None => 0 };",
        );
        assert!(out.contains("__case"), "expected case dispatch, got: {}", out);
        assert!(out.contains("Some"), "got: {}", out);
        assert!(out.contains("None"), "got: {}", out);
    }

    #[test]
    fn emit_enum_constructor() {
        let out = parse_and_emit("const o = Some(5);");
        assert!(out.contains("__case"), "expected case tag, got: {}", out);
        assert!(out.contains("Some"), "got: {}", out);
    }

    #[test]
    fn emit_struct_literal() {
        let out = parse_and_emit(
            "struct Point { x: number\n  y: number }\nconst p = Point { x: 1, y: 2 };",
        );
        assert!(out.contains("Point({"), "expected factory call, got: {}", out);
        assert!(out.contains("x: 1"), "got: {}", out);
        assert!(out.contains("y: 2"), "got: {}", out);
    }

    #[test]
    fn emit_user_defined_enum_constructor() {
        let out = parse_and_emit("enum Color { Red, Green, Blue } const c = Color.Red;");
        assert!(out.contains("const Color = Object.freeze"), "got: {}", out);
        assert!(out.contains("Color.Red"), "got: {}", out);
    }

    #[test]
    fn emit_user_defined_enum_payload_constructor() {
        let out = parse_and_emit("enum Shape { Circle(number) } const s = Shape.Circle(5);");
        assert!(out.contains("Shape.Circle(5)"), "got: {}", out);
    }

    #[test]
    fn emit_receiver_method() {
        let out = parse_and_emit(
            "struct Point { x: number\n  y: number }\nfn (p Point) distance(other: Point) number { return 0; }\nconst p1 = Point { x: 0, y: 0 };\nconst p2 = Point { x: 3, y: 4 };\nconst d = p1.distance(p2);",
        );
        assert!(out.contains("const Point = deka.Struct"), "got: {}", out);
        assert!(out.contains("Point.impl(\"distance\""), "got: {}", out);
        assert!(out.contains("p1.distance(p2)"), "got: {}", out);
    }

    #[test]
    fn emit_import_named() {
        let out = parse_and_emit("import { add } from \"./math.ds\";");
        assert!(out.contains("import { add } from \"./math.ds\";"), "got: {}", out);
    }

    #[test]
    fn emit_import_aliased() {
        let out = parse_and_emit("import { add as plus } from \"./math.ds\";");
        assert!(out.contains("import { add as plus } from \"./math.ds\";"), "got: {}", out);
    }

    #[test]
    fn emit_import_side_effect() {
        let out = parse_and_emit("import \"./side-effects.ds\";");
        assert!(out.contains("import \"./side-effects.ds\";"), "got: {}", out);
    }

    #[test]
    fn emit_export_const() {
        let out = parse_and_emit("export const x: number = 42;");
        assert!(out.contains("export const x = 42;"), "got: {}", out);
    }

    #[test]
    fn emit_export_function() {
        let out = parse_and_emit("export fn add(a: number, b: number) number { return a + b; }");
        assert!(out.contains("export function add(a, b) {"), "got: {}", out);
        assert!(out.contains("return a + b;"), "got: {}", out);
    }

    #[test]
    fn emit_export_named_group() {
        let out = parse_and_emit("const answer = 42; export { answer };");
        assert!(out.contains("export { answer };"), "got: {}", out);
    }

    #[test]
    fn emit_array_object_index() {
        let out = parse_and_emit("const a = [1, 2, 3]; const o = { x: 1 }; const v = a[0] + o[\"x\"];");
        assert!(out.contains("const a = Object.freeze([1, 2, 3]);"), "got: {}", out);
        assert!(out.contains("const o = Object.freeze({x: 1});"), "got: {}", out);
        assert!(out.contains("a[0] + o[\"x\"]"), "got: {}", out);
    }

    #[test]
    fn emit_await_and_pipe() {
        let out = parse_and_emit(
            "async fn fetch() Promise<number> { return 1; } fn double(n: number) number { return n * 2; } const y = await fetch() |> double;",
        );
        assert!(out.contains("await fetch()"), "got: {}", out);
        assert!(out.contains("(double)("), "got: {}", out);
    }

    #[test]
    fn emit_unsafe_expression() {
        let out = parse_and_emit("const r = unsafe { JSON.parse('{}') };");
        assert!(out.contains("__case: \"Ok\""), "got: {}", out);
        assert!(out.contains("JSON.parse('{}')"), "got: {}", out);
    }

    #[test]
    fn emit_unsafe_async_await() {
        let out = parse_and_emit("const r = unsafe { await fetch(url) };");
        assert!(out.contains("async function"), "expected async wrapper, got: {}", out);
        assert!(out.contains("await fetch(url)"), "expected raw await, got: {}", out);
    }

    #[test]
    fn emit_unsafe_statement_block() {
        let out = parse_and_emit("const r = unsafe { const x = 1; return x + 2; };");
        assert!(out.contains("const x = 1;"), "expected raw JS statements, got: {}", out);
        assert!(out.contains("return x + 2;"), "expected raw JS statements, got: {}", out);
    }

    /// Collapse runs of whitespace so wrapper-selection assertions do not
    /// depend on the emitter's line breaks. deka#424 put the body's delimiters
    /// on their own lines to stop a trailing `//` comment swallowing them,
    /// which broke every assertion here that spelled the spacing out.
    fn squeeze(source: &str) -> String {
        source.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn emit_unsafe_ignores_string_punctuation_and_keywords() {
        let out = parse_and_emit("const a = unsafe { \"a;b\" }; const b = unsafe { \"await\" };");
        let flat = squeeze(&out);
        assert!(flat.contains("return ( \"a;b\" )"), "string semicolon changed shape: {}", out);
        assert!(flat.contains("return ( \"await\" )"), "string await changed shape: {}", out);
        assert!(!out.contains("async function"), "string await changed wrapper asyncness: {}", out);
    }

    #[test]
    fn emit_unsafe_ignores_comment_punctuation() {
        let out = parse_and_emit("const r = unsafe { 1 + 1 /* ; await */ };");
        let flat = squeeze(&out);
        assert!(flat.contains("return ( 1 + 1 /* ; await */ )"), "comment changed expression shape: {}", out);
        assert!(!out.contains("async function"), "comment await changed wrapper asyncness: {}", out);
    }

    #[test]
    fn emit_unsafe_ignores_regex_punctuation() {
        let out = parse_and_emit("const r = unsafe { /a;b/.test(value) };");
        let flat = squeeze(&out);
        assert!(flat.contains("return ( /a;b/.test(value) )"), "regex semicolon changed shape: {}", out);
    }

    #[test]
    fn emit_unsafe_detects_automatic_semicolon_insertion() {
        let out = parse_and_emit("const r = unsafe { 1\n2 };");
        let flat = squeeze(&out);
        // Statement wrapper: no `return (`, the body is spliced as statements.
        assert!(flat.contains("function() { 1 2 }"), "ASI statements were treated as an expression: {}", out);
    }

    /// The bug that started deka#423: a body whose last line is a `//` comment
    /// used to swallow the closing delimiters. Both halves are needed -- the
    /// scanner picks the wrapper, deka#424 emits its delimiters on own lines.
    #[test]
    fn emit_unsafe_survives_a_trailing_line_comment() {
        let out = parse_and_emit("const r = unsafe { 1 + 1 // trailing\n };");
        let opens = out.matches('{').count();
        let closes = out.matches('}').count();
        assert_eq!(opens, closes, "unbalanced braces from trailing comment: {}", out);
        assert!(
            out.contains("// trailing\n"),
            "comment must stay on its own line: {}",
            out
        );
    }

    #[test]
    fn emit_jsx_element() {
        let out = parse_and_emit("const el = <div class=\"box\" />;");
        assert!(out.contains("import { jsx, jsxs, Fragment } from \"ui/jsx\""), "got: {}", out);
        assert!(out.contains("jsx("), "expected jsx call, got: {}", out);
        assert!(out.contains("\"div\""), "expected tag, got: {}", out);
        assert!(out.contains("\"class\": \"box\""), "expected class prop, got: {}", out);
    }

    #[test]
    fn emit_jsx_does_not_live_wrap_conditional_elements() {
        let out = parse_and_emit("const el = <div>{cond && <b>hi</b>}</div>;");
        assert!(
            !out.contains("live(function() { return cond &&"),
            "JSX-producing interpolations must not be live() text bindings: {out}"
        );
        assert!(out.contains("cond &&"), "conditional jsx child should still emit: {out}");
    }

    #[test]
    fn emit_jsx_with_children() {
        let out = parse_and_emit("const el = <p>hello {name}</p>;");
        assert!(out.contains("jsxs("), "expected jsxs call, got: {}", out);
        assert!(out.contains("\"children\": ["), "expected children array, got: {}", out);
        assert!(
            out.contains("import { live } from \"ui/reactive\""),
            "non-literal interpolations must import live: {out}"
        );
        assert!(
            out.contains("live(function() { return name; })"),
            "non-literal interpolations must wrap live(): {out}"
        );
    }

    #[test]
    fn emit_skips_css_imports() {
        let out = parse_and_emit("import \"./card.css\";\nconst x = 1;");
        assert!(
            !out.contains("card.css"),
            "CSS imports must not emit JS import: {out}"
        );
        assert!(out.contains("const x = 1;"), "got: {out}");
    }

    #[test]
    fn emit_keeps_css_module_specifier_imports() {
        let out = parse_and_emit("import { styles } from \"./card.module.css\";\nconst x = styles;");
        assert!(
            out.contains("card.module.css"),
            "CSS module specifier imports must stay in the JS graph: {out}"
        );
    }

    #[test]
    fn emit_jsx_client_directive() {
        let out = parse_and_emit("const el = <Cart client:load userId={id} />;");
        assert!(
            out.contains("\"client:load\": true"),
            "namespaced client directive must emit as a prop: {out}"
        );
        assert!(
            out.contains("\"userId\": id"),
            "island props must emit: {out}"
        );
        assert!(
            !out.contains("..."),
            "island emit must not spread props: {out}"
        );
    }

    #[test]
    fn emit_jsx_fragment() {
        let out = parse_and_emit("const el = <><span>a</span><span>b</span></>;");
        assert!(out.contains("jsxs("), "expected jsxs call, got: {}", out);
        assert!(out.contains("Fragment"), "expected Fragment, got: {}", out);
    }

    #[test]
    fn emit_jsx_does_not_concat_html() {
        let out = parse_and_emit("const el = <div class=\"box\" />;");
        assert!(!out.contains("__deka_ui"), "got: {}", out);
        assert!(!out.contains("`<${tag}"), "got: {}", out);
        assert!(out.contains("\"data-deka-id\""), "expected tagged id, got: {}", out);
        assert!(out.contains("i0"), "expected i0 path segment, got: {}", out);
    }

    #[test]
    fn emit_template_literal() {
        let out = parse_and_emit("const s = `hello ${x}`;");
        assert!(out.contains("const s = `hello ${x}`;"), "expected backtick output, got: {}", out);
    }

    #[test]
    fn emit_fn_expression_literal() {
        let out = parse_and_emit("const double = fn (x: number) number { return x * 2 };");
        assert!(out.contains("const double = function(x) {"), "expected function expression, got: {}", out);
        assert!(out.contains("return x * 2;"), "expected return body, got: {}", out);
    }

    #[test]
    fn emit_for_loop() {
        let out = parse_and_emit("for (let i = 0; i < 10; i = i + 1) { break; }");
        assert!(out.contains("for (let i = 0; i < 10; i = i + 1) {"), "expected for header, got: {}", out);
        assert!(out.contains("break;"), "expected break, got: {}", out);
    }

    #[test]
    fn emit_async_function() {
        let out = parse_and_emit("async fn value() Promise<number> { return 1 }");
        assert!(out.contains("async function value()"), "expected async function, got: {}", out);
        assert!(out.contains("return 1;"), "expected return, got: {}", out);
    }

    #[test]
    fn emit_struct_embed_method() {
        let out = parse_and_emit(
            "struct Legs {} fn (l Legs) move() string { return \"walk\" } struct Robot { Legs } const r = Robot { Legs: Legs {} }; const m = r.move();",
        );
        assert!(out.contains("const Legs = deka.Struct"), "got: {}", out);
        assert!(out.contains("const Robot = deka.Struct(\"Robot\", { Legs: Legs })"), "got: {}", out);
        assert!(out.contains("Legs.impl(\"move\""), "got: {}", out);
        assert!(out.contains("r.move()"), "got: {}", out);
    }

    #[test]
    fn emit_struct_embed_promoted_field_literal() {
        // deka#496: promoted fields in a struct literal are routed into the
        // embedded struct's constructor.
        let out = parse_and_emit(
            "struct Person { name: string } struct Employee { Person } const e = Employee { name: \"Bob\" };",
        );
        assert!(
            out.contains("Employee({ Person: Person({ name: \"Bob\" }) })"),
            "got: {}",
            out
        );
    }

    #[test]
    fn emit_struct_embed_nested_promoted_field_literal() {
        let out = parse_and_emit(
            "struct Legs { count: number } struct Robot { Legs } struct Cyborg { Robot } const c = Cyborg { count: 4 };",
        );
        assert!(
            out.contains("Cyborg({ Robot: Robot({ Legs: Legs({ count: 4 }) }) })"),
            "got: {}",
            out
        );
    }
}
