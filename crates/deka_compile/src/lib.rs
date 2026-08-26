//! DekaScript compiler orchestrator (Compiler v2).

use bumpalo::Bump;
use deka_emit::emit_js;
use deka_syntax::{check_program, parse, Diagnostic};

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

/// Successful result of compiling a DekaScript source file to JavaScript.
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
}
