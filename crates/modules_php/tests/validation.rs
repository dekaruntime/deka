use std::fs;
use std::path::{Path, PathBuf};

use bumpalo::Bump;

use modules_php::compiler_api::compile_deka;
use modules_php::validation::imports::validate_imports;
use modules_php::validation::{ErrorKind, ValidationResult};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture(path: &Path) -> String {
    fs::read_to_string(path).expect("fixture read failed")
}

fn compile_fixture(path: &Path) -> ValidationResult<'static> {
    let source = load_fixture(path);
    let arena = Box::leak(Box::new(Bump::new()));
    if path.extension().is_some_and(|ext| ext == "ds") {
        compile_deka(&source, path.to_string_lossy().as_ref(), arena)
    } else {
        compile_deka(&source, path.to_string_lossy().as_ref(), arena)
    }
}

fn compile_source(source: &str, file_path: &str) -> ValidationResult<'static> {
    let arena = Box::leak(Box::new(Bump::new()));
    compile_deka(source, file_path, arena)
}

fn assert_has_error(result: &ValidationResult<'_>, kind: ErrorKind) {
    assert!(
        result.errors.iter().any(|err| err.kind == kind),
        "expected {:?}, got: {:?}",
        kind,
        result.errors
    );
}

fn assert_has_warning(result: &ValidationResult<'_>, kind: ErrorKind) {
    assert!(
        result.warnings.iter().any(|warn| warn.kind == kind),
        "expected warning {:?}, got: {:?}",
        kind,
        result.warnings
    );
}

fn assert_has_error_any(result: &ValidationResult<'_>, kinds: &[ErrorKind]) {
    assert!(
        result.errors.iter().any(|err| kinds.contains(&err.kind)),
        "expected one of {:?}, got: {:?}",
        kinds,
        result.errors
    );
}

#[test]
fn module_import_ok() {
    let path = fixtures_root().join("modules/basic.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_string_subset_fixture_compiles() {
    let path = fixtures_root().join("dekascript/string_subset.ds");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_jsx_component_destructured_props_recognized() {
    // dekaruntime/deka#93 / #122: JSX component props must be validated against
    // the interface type of a destructured object parameter in DekaScript mode.
    let source = "interface GreetingProps { name: string }\nfn Greeting({ name }: GreetingProps): Component {\n  return <h1>Hello {name}</h1>\n}\n---\n<Greeting name=\"DekaScript\" />";
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "lesson.ds", arena);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_fixture_rejects_php_surface() {
    let path = fixtures_root().join("dekascript/php_surface_rejected.ds");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::SyntaxError);
}

#[test]
fn wasm_stub_type_error() {
    let path = fixtures_root().join("wasm/type_error.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn module_missing_module_reports_error() {
    let path = fixtures_root().join("modules/missing_module.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ModuleError);
}

#[test]
fn module_missing_export_reports_error() {
    let path = fixtures_root().join("modules/missing_export.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ModuleError);
}

#[test]
fn import_ok() {
    let path = fixtures_root().join("imports/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    assert!(
        result.warnings.is_empty(),
        "unexpected warnings: {:?}",
        result.warnings
    );
}

#[test]
fn import_default_ok() {
    let path = fixtures_root().join("imports/default_ok.phpx");
    let source = load_fixture(&path);
    let (errors, warnings) = validate_imports(&source, path.to_string_lossy().as_ref());
    assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    assert!(warnings.is_empty(), "unexpected warnings: {:?}", warnings);
}

#[test]
fn import_unused_reports_warning() {
    let path = fixtures_root().join("imports/unused.phpx");
    let result = compile_fixture(&path);
    assert_has_warning(&result, ErrorKind::ImportError);
}

#[test]
fn import_duplicate_reports_error() {
    let path = fixtures_root().join("imports/duplicate.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ImportError);
}

#[test]
fn import_after_code_reports_error() {
    let path = fixtures_root().join("imports/after_code.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ImportError);
}

// `../` relative imports are now allowed (issue #36 fix).  A `../` path that
// points to a file that does not exist still produces an error — but it is a
// ModuleError ("Missing phpx module"), not an ImportError.  This test verifies
// that the error kind changed from ImportError → ModuleError.
#[test]
fn import_relative_path_missing_file_reports_module_error() {
    let path = fixtures_root().join("imports/relative.phpx");
    let result = compile_fixture(&path);
    // The fixture imports `../core/result` which does not exist on disk.
    // After the fix the validator no longer rejects `../` syntax; it falls
    // through to the module resolver which emits a ModuleError.
    assert_has_error(&result, ErrorKind::ModuleError);
    // Confirm the old ImportError is gone.
    assert!(
        !result
            .errors
            .iter()
            .any(|e| e.kind == ErrorKind::ImportError),
        "ImportError for ../relative path should be gone after issue #36 fix"
    );
}

#[test]
fn import_invalid_syntax_reports_error() {
    let path = fixtures_root().join("imports/invalid_syntax.phpx");
    let result = compile_fixture(&path);
    assert_has_error_any(&result, &[ErrorKind::ImportError, ErrorKind::ModuleError]);
}

#[test]
fn import_default_wasm_reports_error() {
    let path = fixtures_root().join("imports/default_wasm.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ImportError);
}

#[test]
fn wasm_example_ok() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/wasm_hello_wit/app.phpx");
    let path = path.canonicalize().expect("example path should resolve");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn export_ok() {
    let path = fixtures_root().join("exports/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn export_async_ok() {
    let path = fixtures_root().join("exports/async_ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn export_undefined_reports_error() {
    let path = fixtures_root().join("exports/undefined.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ExportError);
}

#[test]
fn export_duplicate_reports_error() {
    let path = fixtures_root().join("exports/duplicate.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ExportError);
}

#[test]
fn export_invalid_syntax_reports_error() {
    let path = fixtures_root().join("exports/invalid_syntax.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ExportError);
}

#[test]
fn export_template_reports_error() {
    let path = fixtures_root().join("exports/template_export.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ExportError);
}

#[test]
fn generics_ok() {
    let path = fixtures_root().join("generics/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    assert!(
        result.warnings.is_empty(),
        "unexpected warnings: {:?}",
        result.warnings
    );
}

#[test]
fn generics_unused_reports_warning() {
    let path = fixtures_root().join("generics/unused.phpx");
    let result = compile_fixture(&path);
    assert_has_warning(&result, ErrorKind::TypeError);
}

#[test]
fn syntax_missing_semicolon_reports_error() {
    let path = fixtures_root().join("syntax/missing_semicolon.phpx");
    let result = compile_fixture(&path);
    assert_has_error_any(
        &result,
        &[ErrorKind::SyntaxError, ErrorKind::UnexpectedToken],
    );

    let message = result
        .errors
        .iter()
        .find(|err| {
            matches!(
                err.kind,
                ErrorKind::SyntaxError | ErrorKind::UnexpectedToken
            )
        })
        .map(|err| err.message.as_str())
        .unwrap_or("");

    assert!(
        message.contains("Statements must be separated"),
        "expected PHPX ASI message, got: {message}"
    );
}

#[test]
fn types_ok() {
    let path = fixtures_root().join("types/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn relation_annotation_ok() {
    let path = fixtures_root().join("types/relation_ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn relation_hasmany_non_array_reports_type_error() {
    let path = fixtures_root().join("types/relation_hasmany_non_array.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn relation_belongsto_missing_fk_reports_type_error() {
    let path = fixtures_root().join("types/relation_belongsto_missing_fk.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn structs_ok() {
    let path = fixtures_root().join("structs/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn patterns_ok() {
    let path = fixtures_root().join("patterns/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn jsx_ok() {
    let path = fixtures_root().join("jsx/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn jsx_comparison_requires_spacing() {
    let path = fixtures_root().join("jsx/compare_spacing.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::JsxError);
}

#[test]
fn jsx_server_defer_component_ok() {
    let path = fixtures_root().join("jsx/server_defer_ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn jsx_server_defer_on_dom_tag_reports_error() {
    let path = fixtures_root().join("jsx/server_defer_dom_invalid.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::JsxError);
}

#[test]
fn frontmatter_ok() {
    let path = fixtures_root().join("frontmatter/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn rules_ok() {
    let path = fixtures_root().join("rules/ok.phpx");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn match_missing_case_reports_error() {
    let path = fixtures_root().join("patterns/enum_missing_case.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::PatternError);
}

#[test]
fn match_payload_mismatch_reports_error() {
    let path = fixtures_root().join("patterns/enum_payload_mismatch.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::PatternError);
}

#[test]
fn match_duplicate_case_reports_error() {
    let path = fixtures_root().join("patterns/enum_duplicate_case.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::PatternError);
}

#[test]
fn ds_nested_constructor_patterns_compile() {
    let source = r#"
fn f(r: Result<Option<number>, string>) number {
  return match (r) {
    Err(_) => 0,
    Ok(None) => 1,
    Ok(Some(v)) => v,
  }
}
console.log(f(Ok(Some(7))))
"#;
    let result = compile_source(source, "nested.ds");
    assert!(
        result.errors.is_empty(),
        "Ok(Some(v)) must compile, got: {:?}",
        result.errors
    );
}

#[test]
fn ds_nested_constructor_without_none_arm_compiles() {
    let source = r#"
fn f(r: Result<Option<number>, string>) number {
  return match (r) {
    Err(_) => 0,
    Ok(Some(v)) => v,
    _ => 9,
  }
}
console.log(f(Ok(Some(7))))
"#;
    let result = compile_source(source, "nested_wildcard.ds");
    assert!(
        result.errors.is_empty(),
        "Ok(Some(v)) with a catch-all must compile, got: {:?}",
        result.errors
    );
}

#[test]
fn ds_duplicate_nested_constructor_is_unreachable() {
    let source = r#"
fn f(r: Result<Option<number>, string>) number {
  return match (r) {
    Ok(Some(a)) => a,
    Ok(Some(b)) => b,
    _ => 0,
  }
}
"#;
    let result = compile_source(source, "nested_dup.ds");
    assert!(
        result.errors.iter().any(|err| err.message.contains("Unreachable")),
        "duplicate Ok(Some(_)) must be unreachable, got: {:?}",
        result.errors
    );
}

#[test]
fn rule_null_reports_error() {
    let path = fixtures_root().join("rules/null_value.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn rule_throw_reports_error() {
    let path = fixtures_root().join("rules/throw.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::ExceptionNotAllowed);
}

#[test]
fn rule_class_reports_error() {
    let path = fixtures_root().join("rules/class.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::OopNotAllowed);
}

#[test]
fn dekascript_null_literal_rejected() {
    let source = "const x = null\n";
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "test.ds", arena);
    assert_has_error(&result, ErrorKind::NullNotAllowed);
}

#[test]
fn dekascript_undefined_pseudo_literal_rejected() {
    let source = "const x = undefined\n";
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "test.ds", arena);
    assert_has_error(&result, ErrorKind::UndefinedNotAllowed);
}

#[test]
fn rule_namespace_reports_error() {
    let path = fixtures_root().join("rules/namespace.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::NamespaceNotAllowed);
}

#[test]
fn type_nullable_reports_error() {
    let path = fixtures_root().join("types/nullable_type.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::NullNotAllowed);
}

#[test]
fn type_union_reports_error() {
    let path = fixtures_root().join("types/unsupported_union.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn type_generic_arity_reports_error() {
    let path = fixtures_root().join("types/generic_arity.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn type_mismatch_reports_error() {
    let path = fixtures_root().join("types/type_mismatch.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn struct_missing_field_reports_error() {
    let path = fixtures_root().join("structs/missing_field.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::StructError);
}

#[test]
fn struct_extra_field_reports_error() {
    let path = fixtures_root().join("structs/extra_field.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::StructError);
}

#[test]
fn jsx_unknown_component_reports_error() {
    let path = fixtures_root().join("jsx/unknown_component.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::JsxError);
}

#[test]
fn jsx_island_directive_on_component_ok() {
    let source = r#"
function Card($props) {
    return <div>{$props.message}</div>
}

function Page() {
    return <Card message="hello" clientLoad={true} />
}
"#;
    let result = compile_source(source, "inline/island_ok.phpx");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn jsx_island_directive_on_dom_reports_error() {
    let source = r#"
function Page() {
    return <div clientLoad={true}>x</div>
}
"#;
    let result = compile_source(source, "inline/island_dom.phpx");
    assert_has_error(&result, ErrorKind::JsxError);
}

#[test]
fn jsx_island_media_requires_string() {
    let source = r#"
function Card($props) {
    return <div>{$props.message}</div>
}

function Page() {
    return <Card message="hello" clientMedia={123} />
}
"#;
    let result = compile_source(source, "inline/island_media.phpx");
    assert_has_error(&result, ErrorKind::JsxError);
}

#[test]
fn jsx_invalid_attr_reports_error() {
    let path = fixtures_root().join("jsx/invalid_attr.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::JsxError);
}

#[test]
fn frontmatter_missing_delimiter_reports_error() {
    let path = fixtures_root().join("frontmatter/missing_delimiter.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::JsxError);
}

#[test]
fn frontmatter_invalid_template_reports_error() {
    let path = fixtures_root().join("frontmatter/invalid_template.phpx");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::JsxError);
}

#[test]
fn multiple_errors_collected() {
    let path = fixtures_root().join("multiple_errors.phpx");
    let result = compile_fixture(&path);
    assert!(result.errors.len() >= 2, "expected multiple errors");
    assert_has_error(&result, ErrorKind::ModuleError);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn dekascript_empty_embed_omitted_ok() {
    let path = fixtures_root().join("dekascript/struct_empty_embed_ok.ds");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_nonempty_embed_required_errors() {
    let path = fixtures_root().join("dekascript/struct_nonempty_embed_required.ds");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::StructError);
}

#[test]
fn dekascript_embed_override_ok() {
    let path = fixtures_root().join("dekascript/struct_embed_override_ok.ds");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_embed_ambiguous_errors() {
    let path = fixtures_root().join("dekascript/struct_embed_ambiguous.ds");
    let result = compile_fixture(&path);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn dekascript_optional_field_defaults_to_none_ok() {
    let path = fixtures_root().join("dekascript/struct_optional_field_ok.ds");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_optional_explicit_option_ok() {
    let path = fixtures_root().join("dekascript/struct_optional_explicit_ok.ds");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_match_option_unqualified_ok() {
    let path = fixtures_root().join("dekascript/match_option_unqualified_ok.ds");
    let result = compile_fixture(&path);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_match_number_without_wildcard_is_rejected() {
    let source = r#"
        fn label(n: number) string {
            return match (n) {
                1 => "one",
                2 => "two",
            }
        }
    "#;
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "test.ds", arena);
    assert_has_error(&result, ErrorKind::TypeError);
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.message.contains("not exhaustive") && e.message.contains("`_`")),
        "expected catch-all diagnostic, got: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_enum_non_generic_payload_ok() {
    let source = r#"
        enum Msg { Text(string), Ping }
        fn body(m: Msg) string {
            return match (m) {
                Msg::Text(b) => b,
                Msg::Ping => "ok",
            }
        }
    "#;
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "test.ds", arena);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_enum_type_named_payload_field_is_rejected() {
    let source = r#"
        enum Msg { Text(string), Ping }
        fn body(m: Msg) string {
            return match (m) {
                Msg::Text => m.string,
                Msg::Ping => "ok",
            }
        }
    "#;
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "test.ds", arena);
    assert_has_error(&result, ErrorKind::TypeError);
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.message.contains("bound in the match pattern")),
        "expected pattern-binding diagnostic, got: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_enum_int_named_payload_field_is_rejected() {
    let source = r#"
        enum Msg { Count(int), Ping }
        fn body(m: Msg) number {
            return match (m) {
                Msg::Count => m.int,
                Msg::Ping => 0,
            }
        }
    "#;
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "test.ds", arena);
    assert_has_error(&result, ErrorKind::TypeError);
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.message.contains("bound in the match pattern")),
        "expected pattern-binding diagnostic, got: {:?}",
        result.errors
    );
}

#[test]
fn dekascript_enum_non_generic_payload_mismatch_reports_error() {
    let source = r#"
        enum Msg { Text(string), Ping }
        fn bad() Msg { return Msg::Text(123); }
    "#;
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "test.ds", arena);
    assert_has_error(&result, ErrorKind::TypeError);
}

#[test]
fn dekascript_enum_non_generic_payload_field_outside_arm_errors() {
    let source = r#"
        enum Msg { Text(string), Ping }
        fn body(m: Msg) string {
            return match (m) {
                Msg::Text => "ok",
                Msg::Ping => m.string,
            }
        }
    "#;
    let arena = Box::leak(Box::new(Bump::new()));
    let result = compile_deka(source, "test.ds", arena);
    assert_has_error(&result, ErrorKind::TypeError);
}
