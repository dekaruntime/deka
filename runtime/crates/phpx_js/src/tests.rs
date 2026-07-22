use super::*;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};

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

mod builtin_rewrites;
mod emitter;
mod jsx;
mod pipe;
mod prelude;
mod scope;
