//! DekaScript JavaScript emitter (Compiler v2).
//!
//! Emits reasonably formatted JavaScript from the v2 AST, erasing all type
//! annotations.

mod expr;
mod r#match;
mod stmt;
mod util;

use deka_syntax::Program;

/// Emit JavaScript for a parsed and type-checked program.
pub fn emit_js(program: &Program, _source: &str) -> Result<String, String> {
    let mut out = String::new();
    for (i, stmt) in program.statements.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        stmt::emit_stmt(&mut out, stmt, 0)?;
    }
    Ok(out)
}

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
        assert!(out.contains("null"), "got: {}", out);
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
            "struct Point { x: number, y: number } const p = Point { x: 1, y: 2 };",
        );
        assert!(out.contains("x: 1"), "got: {}", out);
        assert!(out.contains("y: 2"), "got: {}", out);
    }

    #[test]
    fn emit_user_defined_enum_constructor() {
        let out = parse_and_emit("enum Color { Red, Green, Blue } const c = Color.Red;");
        assert!(out.contains("__case"), "expected case tag, got: {}", out);
        assert!(out.contains("Red"), "got: {}", out);
    }

    #[test]
    fn emit_user_defined_enum_payload_constructor() {
        let out = parse_and_emit("enum Shape { Circle(number) } const s = Shape.Circle(5);");
        assert!(out.contains("__case"), "expected case tag, got: {}", out);
        assert!(out.contains("Circle"), "got: {}", out);
        assert!(out.contains("value: 5"), "got: {}", out);
    }

    #[test]
    fn emit_receiver_method() {
        let out = parse_and_emit(
            "struct Point { x: number, y: number } fn (p Point) distance(other: Point): number { return 0; } const p1 = Point { x: 0, y: 0 }; const p2 = Point { x: 3, y: 4 }; const d = p1.distance(p2);",
        );
        assert!(out.contains("function Point_distance"), "got: {}", out);
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
    fn emit_array_literal() {
        let out = parse_and_emit("const a = [1, 2, 3];");
        assert!(out.contains("const a = [1, 2, 3];"), "got: {}", out);
    }

    #[test]
    fn emit_array_spread() {
        let out = parse_and_emit("const a = [...b];");
        assert!(out.contains("const a = [...b];"), "got: {}", out);
    }

    #[test]
    fn emit_object_literal() {
        let out = parse_and_emit("const o = { a: 1, b: \"two\" };");
        assert!(out.contains("const o = {a: 1, b: \"two\"};"), "got: {}", out);
    }

    #[test]
    fn emit_object_spread() {
        let out = parse_and_emit("const o = { ...base, x: 1 };");
        assert!(out.contains("const o = {...base, x: 1};"), "got: {}", out);
    }

    #[test]
    fn emit_index_access() {
        let out = parse_and_emit("const x = arr[0];");
        assert!(out.contains("const x = arr[0];"), "got: {}", out);
    }

    #[test]
    fn emit_await() {
        let out = parse_and_emit("const x = await fetch();");
        assert!(out.contains("const x = await fetch();"), "got: {}", out);
    }

    #[test]
    fn emit_pipe() {
        let out = parse_and_emit("const y = x |> double;");
        assert!(out.contains("const y = (double)(x);"), "got: {}", out);
    }

    #[test]
    fn emit_unsafe_expression() {
        let out = parse_and_emit("const r = unsafe { JSON.parse('{}') };");
        assert!(out.contains("__case: \"Ok\""), "expected Ok case, got: {}", out);
        assert!(out.contains("__case: \"Err\""), "expected Err case, got: {}", out);
        assert!(out.contains("JSON.parse('{}')"), "expected raw JS, got: {}", out);
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
        assert!(out.contains("deka.ui.jsx"), "expected jsx call, got: {}", out);
        assert!(out.contains("\"div\""), "expected tag, got: {}", out);
        assert!(out.contains("\"class\": \"box\""), "expected class prop, got: {}", out);
    }

    #[test]
    fn emit_jsx_with_children() {
        let out = parse_and_emit("const el = <p>hello {name}</p>;");
        assert!(out.contains("deka.ui.jsxs"), "expected jsxs call, got: {}", out);
        assert!(out.contains("\"children\": ["), "expected children array, got: {}", out);
    }

    #[test]
    fn emit_jsx_fragment() {
        let out = parse_and_emit("const el = <><span>a</span><span>b</span></>;");
        assert!(out.contains("deka.ui.jsxs"), "expected jsxs call, got: {}", out);
        assert!(out.contains("deka.ui.Fragment"), "expected Fragment, got: {}", out);
    }
}
