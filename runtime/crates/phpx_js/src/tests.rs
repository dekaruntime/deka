use super::*;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use std::path::Path;

/// Helper: parse PHPX source and emit JS via the subset emitter.
fn phpx_to_js(source: &str) -> Result<String, String> {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let msgs: Vec<&str> = program.errors.iter().map(|e| e.message).collect();
        return Err(format!("parse errors: {}", msgs.join("; ")));
    }
    emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
}

/// Helper: parse PHPX source and emit JS, returning both the JS and any
/// scope-validation warnings.
fn phpx_to_js_with_warnings(source: &str) -> Result<(String, Vec<String>), String> {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let msgs: Vec<&str> = program.errors.iter().map(|e| e.message).collect();
        return Err(format!("parse errors: {}", msgs.join("; ")));
    }
    emit_js_from_ast_with_warnings(&program, source.as_bytes(), SourceModuleMeta::empty())
}

fn ds_to_js(source: &str) -> Result<String, String> {
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Ds);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let msgs: Vec<&str> = program.errors.iter().map(|e| e.message).collect();
        return Err(format!("parse errors: {}", msgs.join("; ")));
    }
    emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
}

#[test]
fn ds_compiles_typed_bare_identifiers_dot_access_and_templates() {
    let source = "const prefix = `hello`; function greet(user: Object): string { user.name; return `${prefix}`; }";
    let js = ds_to_js(source).expect("DekaScript should compile");
    assert!(js.contains("const prefix ="), "missing const: {js}");
    assert!(js.contains("function greet(user)"), "missing function: {js}");
    assert!(js.contains("user.name"), "missing dot access: {js}");
}

#[test]
fn ds_compiler_entry_selects_native_mode_from_extension() {
    let source = "export const answer = 42;";
    let js = crate::compile_phpx_source_to_js(source, Path::new("lesson.ds").to_str().unwrap(), crate::parse_source_module_meta(source)).expect(".ds source should compile through native mode");
    assert!(js.contains("const answer = 42"), "missing emitted const: {js}");
    assert!(js.contains("export { answer };"), "missing ESM export: {js}");
}

mod builtin_rewrites;
mod emitter;
mod jsx;
mod pipe;
mod prelude;
mod scope;
