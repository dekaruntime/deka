use super::*;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};
use std::path::Path;

/// Compiles the core/bytes stdlib module and verifies it lowers to the
/// Uint8Array-backed runtime helpers (RFD 15).
#[test]
fn core_bytes_module_compiles_to_bytes_helpers() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let bytes_module_path = Path::new(manifest_dir)
        .join("../../php_modules/core/bytes.phpx");
    let source = std::fs::read_to_string(&bytes_module_path)
        .expect("core/bytes.phpx should be readable");

    let js = crate::compile_phpx_source_to_js(
        &source,
        "php_modules/core/bytes.phpx",
        SourceModuleMeta::empty(),
    )
    .expect("core/bytes.phpx should compile");

    assert!(
        js.contains("__deka_bytes_from_string"),
        "expected __deka_bytes_from_string in emitted core/bytes: {js}"
    );
    assert!(
        js.contains("__deka_bytes_len"),
        "expected __deka_bytes_len in emitted core/bytes: {js}"
    );
    assert!(
        js.contains("__deka_bytes_concat"),
        "expected __deka_bytes_concat in emitted core/bytes: {js}"
    );
}

/// Verifies a small PHPX snippet using the bytes type annotation compiles
/// and the emitter installs the bytes helpers in its prelude.
#[test]
fn bytes_type_snippet_emits_helpers_in_prelude() {
    let source = r#"
function encode($input: bytes): bytes {
    return $input;
}
"#;
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(program.errors.is_empty(), "parse errors: {:?}", program.errors);

    let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
        .expect("bytes snippet should emit JS");
    assert!(
        js.contains("__deka_bytes_from_string"),
        "bytes helpers missing from prelude: {js}"
    );
}
