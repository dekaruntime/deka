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
    assert!(
        js.contains("function greet(user)"),
        "missing function: {js}"
    );
    assert!(js.contains("user.name"), "missing dot access: {js}");
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
        js.contains("const answer = 42"),
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
        export function joinWords(parts: Array<string>): string {
            let output = "";
            for (const part of parts) {
                output += part.slice(0, 1);
            }
            const first = parts[0];
            const meta = { first: first, count: parts.length };
            return `${first}:${output}`;
        }
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
        js.contains("const meta = {"),
        "missing object literal: {js}"
    );
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
        ("class Legacy {}", "PHP/PHPX construct is not part"),
        ("namespace Legacy;", "PHP/PHPX construct is not part"),
        ("global value;", "PHP/PHPX construct is not part"),
        ("static value;", "PHP/PHPX construct is not part"),
        ("try {} catch (error) {}", "PHP/PHPX construct is not part"),
        ("throw value;", "PHP/PHPX construct is not part"),
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

mod builtin_rewrites;
mod emitter;
mod jsx;
mod pipe;
mod prelude;
mod scope;
