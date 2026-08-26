//! DekaScript compiler orchestrator (Compiler v2).

use bumpalo::Bump;
use deka_emit::emit_js;
use deka_syntax::{check_program, lower_method_calls, parse, Diagnostic};

/// Compiler pipeline version selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilerVersion {
    V1,
    V2,
}

/// Options controlling the compile pipeline.
pub struct CompileOptions {
    pub compiler: CompilerVersion,
}

/// Module metadata extracted from a DekaScript source file.
///
/// This is the v2 equivalent of `deka_js::SourceModuleMeta`. It is intentionally
/// minimal while the v2 module system is being implemented; frontmatter parsing
/// will populate the fields as imports/exports land.
#[derive(Debug, Clone, Default)]
pub struct SourceModuleMeta {
    pub imports: Vec<ImportDecl>,
    pub exports: Vec<ExportDecl>,
}

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub path: String,
    pub specs: Vec<ImportSpec>,
}

#[derive(Debug, Clone)]
pub struct ImportSpec {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExportDecl {
    pub name: String,
}

/// Parse frontmatter metadata from a DekaScript source file.
///
/// Currently returns an empty metadata object; frontmatter parsing will be
/// wired up once the v2 module syntax is stable.
pub fn parse_source_module_meta(_source: &str) -> SourceModuleMeta {
    SourceModuleMeta::default()
}

/// Successful result of compiling a DekaScript source file to JavaScript.
#[derive(Debug)]
pub struct CompileResult {
    pub js: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Compile a DekaScript source to JavaScript using the v2 pipeline.
///
/// The pipeline is: parse -> typecheck -> emit.  If parsing or typechecking
/// produce errors they are returned directly.  Emit errors are converted to a
/// single diagnostic.
pub fn compile_to_js(source: &str, file_path: &str) -> Result<CompileResult, Vec<Diagnostic>> {
    let arena = Bump::new();

    let parse_result = parse(source, &arena);
    if !parse_result.errors.is_empty() {
        return Err(parse_result.errors);
    }

    let program = parse_result.program.ok_or_else(|| {
        vec![Diagnostic::error(
            0,
            0,
            format!("parse produced no program for {}", file_path),
        )]
    })?;

    let typeck_result = check_program(&program, source);
    if !typeck_result.errors.is_empty() {
        return Err(typeck_result.errors);
    }

    // Lower method calls after typechecking so the emitter sees ordinary
    // function calls instead of struct receiver syntax.
    let mut program = program.clone();
    lower_method_calls(&mut program, &arena, &typeck_result.method_calls);

    let js = emit_js(&program, source).map_err(|message| vec![Diagnostic::error(0, 0, message)])?;

    Ok(CompileResult {
        js,
        diagnostics: typeck_result.warnings,
    })
}

/// Compile a DekaScript source to JavaScript, returning only the emitted JS.
///
/// This is a convenience wrapper around [`compile_to_js`] that formats any
/// diagnostics into a single string on failure.
pub fn compile(source: &str, path: &str) -> Result<String, String> {
    compile_to_js(source, path)
        .map(|result| result.js)
        .map_err(|diagnostics| format_diagnostics(&diagnostics))
}

/// Format a diagnostic in a stable, human-readable form.
pub fn format_diagnostic(diagnostic: &Diagnostic) -> String {
    format!("{}:{}: {}", diagnostic.line, diagnostic.column, diagnostic.message)
}

/// Format a list of diagnostics into a single multi-line string.
pub fn format_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(format_diagnostic)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_const_number() {
        let result = compile_to_js("const x = 42;", "test.ds").expect("compile should succeed");
        assert!(
            result.js.contains("const x = 42;"),
            "expected emitted JS to contain 'const x = 42;', got:\n{}",
            result.js
        );
    }

    #[test]
    fn compile_function_and_call() {
        let result = compile_to_js(
            "function add(a: number, b: number): number { return a + b; } const r = add(1, 2);",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("function add"));
        assert!(result.js.contains("add(1, 2)"));
    }

    #[test]
    fn compile_recursive_function() {
        let result = compile_to_js(
            "function forever(n: number): number { return forever(n); }",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("function forever"));
    }

    #[test]
    fn compile_option_none() {
        let result = compile_to_js("const x: Option<number> = none;", "test.ds")
            .expect("compile should succeed");
        assert!(result.js.contains("const x"));
    }

    #[test]
    fn compile_type_error_returns_diagnostics() {
        let err = compile_to_js("const x: string = 42;", "test.ds").expect_err("compile should fail");
        assert!(
            err.iter().any(|d| d.message.contains("string") && d.message.contains("number")),
            "expected type mismatch diagnostic, got: {:?}",
            err
        );
    }

    #[test]
    fn compile_match_expression() {
        let result = compile_to_js(
            "const o = Some(5); const x = match o { Some(n) => n, None => 0 };",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("__case"));
        assert!(result.js.contains("Some"));
        assert!(result.js.contains("None"));
    }

    #[test]
    fn compile_struct_literal() {
        let result = compile_to_js(
            "struct Point { x: number, y: number } const p = Point { x: 1, y: 2 };",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("x: 1"));
        assert!(result.js.contains("y: 2"));
    }

    #[test]
    fn compile_user_defined_enum_constructor() {
        let result = compile_to_js(
            "enum Color { Red, Green, Blue } const c: Color = Color.Red;",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("__case"));
        assert!(result.js.contains("Red"));
    }

    #[test]
    fn compile_receiver_method() {
        let result = compile_to_js(
            "struct Point { x: number, y: number } fn Point.distance(other: Point): number { return 0; } const p1: Point = Point { x: 0, y: 0 }; const p2: Point = Point { x: 3, y: 4 }; const d: number = p1.distance(p2);",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("function Point_distance"));
        assert!(result.js.contains("Point_distance(p1, p2)"));
    }

    #[test]
    fn compile_generic_function() {
        let result = compile_to_js(
            "function id<T>(x: T): T { return x; } const n: number = id(5);",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("function id"));
        assert!(result.js.contains("id(5)"));
    }

    #[test]
    fn compile_import_and_use() {
        let result = compile_to_js(
            "import { add } from \"./math.ds\"; const r: number = add(1, 2);",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("import { add } from \"./math.ds\";"), "got: {}", result.js);
        assert!(result.js.contains("add(1, 2)"), "got: {}", result.js);
    }

    #[test]
    fn compile_export_const() {
        let result = compile_to_js(
            "export const x: number = 42;",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("export const x = 42;"), "got: {}", result.js);
    }

    #[test]
    fn compile_export_function() {
        let result = compile_to_js(
            "export function add(a: number, b: number): number { return a + b; }",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("export function add(a, b) {"), "got: {}", result.js);
    }
}
