//! DekaScript compiler orchestrator (Compiler v2).

pub mod module_graph;

use std::collections::HashMap;

use bumpalo::Bump;
use deka_emit::emit_js_with_imports;
use deka_syntax::{check_program_with_imports, parse, resolve_imported_enum_constructors, Diagnostic, ModuleExports};

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
/// This is the v2 equivalent of `deka_js::SourceModuleMeta`. It is populated by
/// parsing import/export statements at the top level of a `.ds` file. There is
/// no frontmatter stage (RFD 24).
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

/// Extract module metadata (imports and exports) from a `.ds` source file.
///
/// This performs a lightweight parse and walks the top-level statements to
/// collect import sources/specifiers and exported names. It does not
/// typecheck or emit. RFD 24: there is no frontmatter stage.
pub fn parse_source_module_meta(source: &str) -> SourceModuleMeta {
    let arena = Bump::new();
    let result = parse(source, &arena);
    let mut imports = Vec::new();
    let mut exports = Vec::new();

    let Some(program) = result.program else {
        return SourceModuleMeta { imports, exports };
    };

    for stmt in program.statements.iter() {
        match stmt {
            deka_syntax::Stmt::Import { specifiers, source: src, .. } => {
                let specs = specifiers
                    .iter()
                    .map(|spec| ImportSpec {
                        name: spec.imported.to_string(),
                        alias: if spec.imported == spec.local {
                            None
                        } else {
                            Some(spec.local.to_string())
                        },
                    })
                    .collect();
                imports.push(ImportDecl {
                    path: src.to_string(),
                    specs,
                });
            }
            deka_syntax::Stmt::Export { decl, .. } => {
                match decl {
                    deka_syntax::ExportDecl::Const { name, .. } => {
                        exports.push(ExportDecl { name: name.to_string() });
                    }
                    deka_syntax::ExportDecl::Function { name, .. } => {
                        exports.push(ExportDecl { name: name.to_string() });
                    }
                    deka_syntax::ExportDecl::NamedGroup { names } => {
                        for name in names.iter() {
                            exports.push(ExportDecl {
                                name: name.alias.unwrap_or(name.name).to_string(),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    SourceModuleMeta { imports, exports }
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
    let imports = HashMap::new();
    compile_to_js_with_imports(source, file_path, &arena, &imports)
}

/// Compile a DekaScript source to JavaScript with imported module signatures.
///
/// The `arena` must outlive any `ModuleExports` stored in `imports` because the
/// returned `CompileResult` does not own the AST.
pub fn compile_to_js_with_imports<'a>(
    source: &str,
    file_path: &str,
    arena: &'a Bump,
    imports: &HashMap<&str, &ModuleExports<'a>>,
) -> Result<CompileResult, Vec<Diagnostic>> {
    let parse_result = parse(source, arena);
    if !parse_result.errors.is_empty() {
        return Err(parse_result.errors);
    }

    let mut program = parse_result.program.ok_or_else(|| {
        vec![Diagnostic::error(
            0,
            0,
            format!("parse produced no program for {}", file_path),
        )]
    })?;

    resolve_imported_enum_constructors(&mut program, arena, imports);

    let typeck_result = check_program_with_imports(&program, source, imports);
    if !typeck_result.errors.is_empty() {
        return Err(typeck_result.errors);
    }

    let js = emit_js_with_imports(&program, source, imports)
        .map_err(|message| vec![Diagnostic::error(0, 0, message)])?;

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
            "fn add(a: number, b: number): number { return a + b; } const r = add(1, 2);",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("function add"));
        assert!(result.js.contains("add(1, 2)"));
    }

    #[test]
    fn compile_recursive_function() {
        let result = compile_to_js(
            "fn forever(n: number): number { return forever(n); }",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("function forever"));
    }

    #[test]
    fn compile_option_none() {
        let result = compile_to_js("const x: Option<number> = None;", "test.ds")
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
            "struct Point { x: number, y: number } fn (p Point) distance(other: Point): number { return 0; } const p1: Point = Point { x: 0, y: 0 }; const p2: Point = Point { x: 3, y: 4 }; const d: number = p1.distance(p2);",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("const Point = deka.Struct"), "got: {}", result.js);
        assert!(result.js.contains("Point.impl(\"distance\""), "got: {}", result.js);
        assert!(result.js.contains("p1.distance(p2)"), "got: {}", result.js);
    }

    #[test]
    fn compile_generic_function() {
        let result = compile_to_js(
            "fn id<T>(x: T): T { return x; } const n: number = id(5);",
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
            "export fn add(a: number, b: number): number { return a + b; }",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("export function add(a, b) {"), "got: {}", result.js);
    }

    #[test]
    fn compile_export_named_group() {
        let result = compile_to_js(
            "const answer = 42; export { answer };",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("export { answer };"), "got: {}", result.js);
    }

    #[test]
    fn compile_array_object_index() {
        let result = compile_to_js(
            "const a = [1, 2, 3]; const o = { x: 1 }; const v = a[0] + o[\"x\"];",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("const a = Object.freeze([1, 2, 3]);"), "got: {}", result.js);
        assert!(result.js.contains("const o = Object.freeze({x: 1});"), "got: {}", result.js);
        assert!(result.js.contains("a[0] + o[\"x\"]"), "got: {}", result.js);
    }

    #[test]
    fn compile_await_and_pipe() {
        let result = compile_to_js(
            "async fn fetch() Promise<number> { return 1; } fn double(n: number): number { return n * 2; } const y = await fetch() |> double;",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("await fetch()"), "got: {}", result.js);
        assert!(result.js.contains("(double)("), "got: {}", result.js);
    }

    #[test]
    fn compile_unsafe_expression() {
        let result = compile_to_js("const r = unsafe { JSON.parse('{}') };", "test.ds")
            .expect("compile should succeed");
        assert!(result.js.contains("__case: \"Ok\""), "got: {}", result.js);
        assert!(result.js.contains("JSON.parse('{}')"), "got: {}", result.js);
    }

    #[test]
    fn compile_unsafe_async() {
        let result = compile_to_js("const r = unsafe { await fetch(url) };", "test.ds")
            .expect("compile should succeed");
        assert!(result.js.contains("async function"), "got: {}", result.js);
        assert!(result.js.contains("await fetch(url)"), "got: {}", result.js);
    }

    #[test]
    fn compile_struct_embed_method() {
        let result = compile_to_js(
            "struct Legs {} fn (l Legs) move() string { return \"walk\" } struct Robot { Legs } const r = Robot { Legs: Legs {} }; const m = r.move();",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("const Legs = deka.Struct"), "got: {}", result.js);
        assert!(result.js.contains("const Robot = deka.Struct(\"Robot\", { Legs: Legs })"), "got: {}", result.js);
        assert!(result.js.contains("Legs.impl(\"move\""), "got: {}", result.js);
        assert!(result.js.contains("r.move()"), "got: {}", result.js);
    }

    #[test]
    fn compile_top_level_await() {
        let result = compile_to_js(
            "async fn main() Promise<number> { return 1 } const n = await main();",
            "test.ds",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("await main()"), "got: {}", result.js);
    }

    #[test]
    fn compile_jsx_element() {
        let result = compile_to_js("const el = <div class=\"box\" />;", "test.dsx")
            .expect("compile should succeed");
        assert!(result.js.contains("deka.ui.jsx"), "got: {}", result.js);
    }

    #[test]
    fn compile_jsx_fragment() {
        let result = compile_to_js("const el = <><span>a</span><span>b</span></>;", "test.dsx")
            .expect("compile should succeed");
        assert!(result.js.contains("deka.ui.Fragment"), "got: {}", result.js);
    }

    #[test]
    fn compile_jsx_component() {
        let result = compile_to_js(
            "const Greeting = fn () { return <h1 /> }; const el = <Greeting name=\"Deka\" />;",
            "test.dsx",
        )
        .expect("compile should succeed");
        assert!(result.js.contains("deka.ui.jsx(Greeting"), "got: {}", result.js);
        assert!(result.js.contains("\"name\": \"Deka\""), "got: {}", result.js);
    }

    #[test]
    fn extract_module_meta() {
        let meta = parse_source_module_meta(
            "import { add } from \"./math.ds\";\nexport const x: number = 1;\nexport fn double(n: number): number { return n * 2; }",
        );
        assert_eq!(meta.imports.len(), 1);
        assert_eq!(meta.imports[0].path, "./math.ds");
        assert_eq!(meta.imports[0].specs.len(), 1);
        assert_eq!(meta.imports[0].specs[0].name, "add");
        assert!(meta.imports[0].specs[0].alias.is_none());
        assert_eq!(meta.exports.len(), 2);
        assert_eq!(meta.exports[0].name, "x");
        assert_eq!(meta.exports[1].name, "double");
    }
}
