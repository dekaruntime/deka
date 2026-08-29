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

    #[test]
    fn emit_const_number() {
        let out = parse_and_emit("const x = 42;");
        assert!(out.contains("const x = 42;"), "got: {}", out);
    }

    #[test]
    fn emit_function_with_return() {
        let out = parse_and_emit("fn add(a: number, b: number): number { return a + b; }");
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
            "struct Point { x: number\n  y: number }\nfn (p Point) distance(other: Point): number { return 0; }\nconst p1 = Point { x: 0, y: 0 };\nconst p2 = Point { x: 3, y: 4 };\nconst d = p1.distance(p2);",
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
        let out = parse_and_emit("export fn add(a: number, b: number): number { return a + b; }");
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
            "async fn fetch() Promise<number> { return 1; } fn double(n: number): number { return n * 2; } const y = await fetch() |> double;",
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

    #[test]
    fn emit_jsx_element() {
        let out = parse_and_emit("const el = <div class=\"box\" />;");
        assert!(out.contains("__deka_ui.jsx"), "expected jsx call, got: {}", out);
        assert!(out.contains("\"div\""), "expected tag, got: {}", out);
        assert!(out.contains("\"class\": \"box\""), "expected class prop, got: {}", out);
    }

    #[test]
    fn emit_jsx_with_children() {
        let out = parse_and_emit("const el = <p>hello {name}</p>;");
        assert!(out.contains("__deka_ui.jsxs"), "expected jsxs call, got: {}", out);
        assert!(out.contains("\"children\": ["), "expected children array, got: {}", out);
    }

    #[test]
    fn emit_jsx_fragment() {
        let out = parse_and_emit("const el = <><span>a</span><span>b</span></>;");
        assert!(out.contains("__deka_ui.jsxs"), "expected jsxs call, got: {}", out);
        assert!(out.contains("__deka_ui.Fragment"), "expected Fragment, got: {}", out);
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
}
