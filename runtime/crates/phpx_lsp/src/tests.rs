use super::*;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{}_{}", prefix, nonce));
    fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn parses_import_module_path_with_span() {
    let line = "import { query } from 'db/postgres'";
    let (module, span) = parse_module_path_with_span(line, 0).expect("module span");
    assert_eq!(module, "db/postgres");
    assert_eq!(&line[span.start..span.end], "db/postgres");
}

#[test]
fn detects_import_module_at_cursor_offset() {
    let src = "import { query } from 'db/postgres'\n$query = 1\n";
    let offset = src.find("postgres").expect("postgres");
    let module = import_module_at_offset(src, offset).expect("module");
    assert_eq!(module, "db/postgres");
    let non_import = src.find("$query").expect("query var");
    assert!(import_module_at_offset(src, non_import).is_none());
}

#[test]
fn collects_all_matching_import_module_spans() {
    let src = "import { a } from 'db/postgres'\nimport { b } from 'db/mysql'\nimport { c } from 'db/postgres'\n";
    let spans = import_module_spans(src, "db/postgres");
    assert_eq!(spans.len(), 2);
    assert_eq!(&src[spans[0].start..spans[0].end], "db/postgres");
    assert_eq!(&src[spans[1].start..spans[1].end], "db/postgres");
}

#[test]
fn target_mode_defaults_to_server() {
    let params = InitializeParams::default();
    assert_eq!(
        TargetMode::from_initialize_params(&params),
        TargetMode::Server
    );
}

#[test]
fn target_mode_reads_adwa_from_init_options() {
    let mut params = InitializeParams::default();
    params.initialization_options = Some(json!({
        "phpx": {
            "target": "adwa"
        }
    }));
    assert_eq!(
        TargetMode::from_initialize_params(&params),
        TargetMode::Adwa
    );
}

#[test]
fn target_capability_diagnostics_block_db_modules_for_adwa() {
    let source = "import { query } from 'db/postgres'\n";
    let diagnostics = target_capability_diagnostics(source, TargetMode::Adwa);
    assert_eq!(diagnostics.len(), 1, "diagnostics={diagnostics:?}");
    let first = &diagnostics[0];
    assert!(
        first.message.contains("db/postgres"),
        "message={}",
        first.message
    );
    assert!(first.message.contains("help:"), "message={}", first.message);
    assert_eq!(
        first.code,
        Some(tower_lsp::lsp_types::NumberOrString::String(
            "Target Capability Error".to_string()
        ))
    );
}

#[test]
fn target_capability_diagnostics_allow_db_modules_for_server() {
    let source = "import { query } from 'db/postgres'\n";
    let diagnostics = target_capability_diagnostics(source, TargetMode::Server);
    assert!(diagnostics.is_empty(), "diagnostics={diagnostics:?}");
}

#[test]
fn finds_whole_word_occurrences_only() {
    let src = b"foo food foo\nfoo_bar foo\n";
    let spans = find_word_occurrences(src, "foo");
    let ranges: Vec<(usize, usize)> = spans.into_iter().map(|s| (s.start, s.end)).collect();
    assert_eq!(ranges, vec![(0, 3), (9, 12), (21, 24)]);
}

#[test]
fn collects_module_rename_edits_across_workspace_files() {
    let dir = temp_dir("phpx_lsp_module_rename");
    let file_a = dir.join("a.phpx");
    let file_b = dir.join("b.phpx");
    let src_a = "import { query } from 'db/postgres'\n";
    let src_b = "import { exec } from 'db/postgres'\n";
    fs::write(&file_a, src_a).expect("write a");
    fs::write(&file_b, src_b).expect("write b");

    let uri_a = Url::from_file_path(&file_a).expect("uri a");
    let edits = collect_module_rename_edits(
        std::slice::from_ref(&dir),
        &uri_a,
        src_a,
        "db/postgres",
        "db/mysql",
    );
    assert_eq!(edits.len(), 2);
    let uri_b = Url::from_file_path(&file_b).expect("uri b");
    assert_eq!(edits.get(&uri_a).map(|v| v.len()), Some(1));
    assert_eq!(edits.get(&uri_b).map(|v| v.len()), Some(1));
}

#[test]
fn collects_symbol_rename_edits_with_word_boundaries_across_files() {
    let dir = temp_dir("phpx_lsp_symbol_rename");
    let file_a = dir.join("a.phpx");
    let file_b = dir.join("b.phpx");
    let src_a = "$foo = 1\n$food = 2\n";
    let src_b = "function run($foo) { return $foo }\n";
    fs::write(&file_a, src_a).expect("write a");
    fs::write(&file_b, src_b).expect("write b");

    let uri_a = Url::from_file_path(&file_a).expect("uri a");
    let edits =
        collect_symbol_rename_edits(std::slice::from_ref(&dir), &uri_a, src_a, "$foo", "$bar");
    let uri_b = Url::from_file_path(&file_b).expect("uri b");
    assert_eq!(edits.get(&uri_a).map(|v| v.len()), Some(1));
    assert_eq!(edits.get(&uri_b).map(|v| v.len()), Some(2));
}

#[test]
fn collects_references_across_workspace_files() {
    let dir = temp_dir("phpx_lsp_refs");
    let file_a = dir.join("a.phpx");
    let file_b = dir.join("b.phpx");
    let src_a = "function run($user) { return $user }\n";
    let src_b = "$user = 'sami'\n";
    fs::write(&file_a, src_a).expect("write a");
    fs::write(&file_b, src_b).expect("write b");

    let uri_a = Url::from_file_path(&file_a).expect("uri a");
    let refs = collect_reference_locations(std::slice::from_ref(&dir), &uri_a, src_a, "$user");
    assert_eq!(refs.len(), 3);
}

#[test]
fn provides_annotation_completion_items() {
    let src = "struct User {\n    $id: int @\n}\n";
    let offset = src.find('@').expect("annotation") + 1;
    let items = completion_for_annotation(src, offset).expect("annotation completion");
    assert!(items.iter().any(|item| item.label == "@autoIncrement"));
    assert!(items.iter().any(|item| item.label == "@relation"));
}

#[test]
fn provides_annotation_hover_docs() {
    let src = "struct User { $id: int @autoIncrement; }";
    let offset = src.find("autoIncrement").expect("annotation");
    let hover = hover_for_annotation(src, offset).expect("annotation hover");
    assert!(hover.contains("@autoIncrement"));
    assert!(hover.contains("Requires an `int` field"));
}

#[test]
fn resolves_project_alias_module_file() {
    let dir = temp_dir("phpx_lsp_alias_resolve");
    let php_modules = dir.join("php_modules");
    let db = dir.join("db");
    fs::create_dir_all(&php_modules).expect("mkdir php_modules");
    fs::create_dir_all(&db).expect("mkdir db");
    fs::write(db.join("index.phpx"), "export const x = 1").expect("write module");

    let resolved = resolve_module_file(&php_modules, "@/db", false).expect("resolve alias");
    assert_eq!(resolved, db.join("index.phpx"));
}

#[test]
fn finds_php_modules_from_workspace_roots_fallback() {
    let workspace = temp_dir("phpx_lsp_workspace_modules");
    let php_modules = workspace.join("php_modules");
    let project = workspace.join("apps").join("sample");
    let file = project.join("main.phpx");
    fs::create_dir_all(&php_modules).expect("mkdir php_modules");
    fs::create_dir_all(&project).expect("mkdir project");
    fs::write(&file, "import { x } from 'core/result'").expect("write file");

    let resolved =
        find_php_modules_root(&file, std::slice::from_ref(&workspace)).expect("resolve modules");
    assert_eq!(resolved, php_modules);
}

#[test]
fn completes_named_exports_for_import_clause() {
    let workspace = temp_dir("phpx_lsp_import_exports");
    let php_modules = workspace.join("php_modules");
    let db = php_modules.join("db");
    fs::create_dir_all(&db).expect("mkdir db");
    fs::write(
        db.join("index.phpx"),
        "export function stats() {}\nexport function status() {}\n",
    )
    .expect("write module");
    let file = workspace.join("main.phpx");
    fs::write(&file, "import { sta } from 'db'\n").expect("write main");

    let source = fs::read_to_string(&file).expect("read main");
    let offset = source.find("sta").expect("sta") + 3;
    let items = completion_for_import(
        &source,
        file.to_str().expect("file"),
        offset,
        std::slice::from_ref(&workspace),
    )
    .expect("completion");
    let labels: Vec<String> = items.into_iter().map(|item| item.label).collect();
    assert!(
        labels.iter().any(|label| label == "stats"),
        "labels={labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "status"),
        "labels={labels:?}"
    );
}

#[test]
fn completes_named_exports_without_closing_brace() {
    let workspace = temp_dir("phpx_lsp_import_partial");
    let php_modules = workspace.join("php_modules");
    let db = php_modules.join("db");
    fs::create_dir_all(&db).expect("mkdir db");
    fs::write(db.join("index.phpx"), "export function stats() {}\n").expect("write module");
    let file = workspace.join("main.phpx");
    let source = "import { sta from 'db'\n";

    let offset = source.find("sta").expect("sta") + 3;
    let items = completion_for_import(
        source,
        file.to_str().expect("file"),
        offset,
        std::slice::from_ref(&workspace),
    )
    .expect("completion");
    let labels: Vec<String> = items.into_iter().map(|item| item.label).collect();
    assert!(
        labels.iter().any(|label| label == "stats"),
        "labels={labels:?}"
    );
}

#[test]
fn completes_jsx_props_from_interface_shape() {
    let source = "$v = <FullName />;\n";
    let index = SymbolIndex {
        functions: vec![FunctionInfo {
            name: "FullName".to_string(),
            span: Span::new(0, 8),
            signature: "function FullName($props: NameProps): string".to_string(),
            props_type: Some("NameProps".to_string()),
            vars: Vec::new(),
            scope_span: Span::new(0, source.len()),
        }],
        interfaces: vec![InterfaceInfo {
            name: "NameProps".to_string(),
            span: Span::new(0, 9),
            fields: vec![
                FieldInfo {
                    name: "$name".to_string(),
                    span: Span::new(0, 5),
                    ty: Some("string".to_string()),
                },
                FieldInfo {
                    name: "$title".to_string(),
                    span: Span::new(0, 6),
                    ty: Some("string".to_string()),
                },
                FieldInfo {
                    name: "$age".to_string(),
                    span: Span::new(0, 4),
                    ty: Some("int".to_string()),
                },
            ],
        }],
        ..SymbolIndex::default()
    };
    let offset = source.find("/>").expect("/>");
    let items = completion_for_jsx_props(&index, source.as_bytes(), offset).expect("completion");
    let labels: Vec<String> = items.into_iter().map(|item| item.label).collect();
    assert!(
        labels.iter().any(|label| label == "name"),
        "labels={labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "title"),
        "labels={labels:?}"
    );
    let items = completion_for_jsx_props(&index, source.as_bytes(), offset).expect("completion");
    let name_item = items
        .iter()
        .find(|item| item.label == "name")
        .expect("name item");
    assert_eq!(name_item.detail.as_deref(), Some("name: string"));
    assert_eq!(name_item.insert_text.as_deref(), Some("name=\"\""));
    let age_item = items
        .iter()
        .find(|item| item.label == "age")
        .expect("age item");
    assert_eq!(age_item.insert_text.as_deref(), Some("age={0}"));
}

#[test]
fn jsx_props_completion_skips_already_used_props() {
    let source = "$v = <FullName name=\"Bob\" />;\n";
    let index = SymbolIndex {
        functions: vec![FunctionInfo {
            name: "FullName".to_string(),
            span: Span::new(0, 8),
            signature: "function FullName($props: NameProps): string".to_string(),
            props_type: Some("NameProps".to_string()),
            vars: Vec::new(),
            scope_span: Span::new(0, source.len()),
        }],
        interfaces: vec![InterfaceInfo {
            name: "NameProps".to_string(),
            span: Span::new(0, 9),
            fields: vec![
                FieldInfo {
                    name: "$name".to_string(),
                    span: Span::new(0, 5),
                    ty: Some("string".to_string()),
                },
                FieldInfo {
                    name: "$title".to_string(),
                    span: Span::new(0, 6),
                    ty: Some("string".to_string()),
                },
            ],
        }],
        ..SymbolIndex::default()
    };
    let offset = source.find("/>").expect("/>");
    let items = completion_for_jsx_props(&index, source.as_bytes(), offset).expect("completion");
    let labels: Vec<String> = items.into_iter().map(|item| item.label).collect();
    assert!(
        !labels.iter().any(|label| label == "name"),
        "labels={labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "title"),
        "labels={labels:?}"
    );
}

#[test]
fn reports_missing_named_import_export() {
    let workspace = temp_dir("phpx_lsp_missing_export");
    let php_modules = workspace.join("php_modules");
    let db = php_modules.join("db");
    fs::create_dir_all(&db).expect("mkdir db");
    fs::write(db.join("index.phpx"), "export function stats() {}\n").expect("write module");
    let file = workspace.join("main.phpx");
    let source = "import { stat } from 'db'\n";
    fs::write(&file, source).expect("write main");

    let diagnostics = unresolved_import_diagnostics(
        source,
        file.to_str().expect("file"),
        std::slice::from_ref(&workspace),
    );
    assert_eq!(diagnostics.len(), 1, "diagnostics={diagnostics:?}");
    assert!(diagnostics[0].message.contains("no export named 'stat'"));
    assert_eq!(
        diagnostics[0].code,
        Some(tower_lsp::lsp_types::NumberOrString::String(
            "Import Error".to_string()
        ))
    );
}

#[test]
fn accepts_valid_named_import_alias() {
    let workspace = temp_dir("phpx_lsp_import_alias_ok");
    let php_modules = workspace.join("php_modules");
    let db = php_modules.join("db");
    fs::create_dir_all(&db).expect("mkdir db");
    fs::write(db.join("index.phpx"), "export function stats() {}\n").expect("write module");
    let file = workspace.join("main.phpx");
    let source = "import { stats as stat } from 'db'\n";
    fs::write(&file, source).expect("write main");

    let diagnostics = unresolved_import_diagnostics(
        source,
        file.to_str().expect("file"),
        std::slice::from_ref(&workspace),
    );
    assert!(diagnostics.is_empty(), "diagnostics={diagnostics:?}");
}

#[test]
fn diagnostics_reject_struct_typed_destructured_props_with_guidance() {
    let source = r#"
interface Ignored {}
struct NameProps { $name: string }
function FullName({ $name }: NameProps): string {
  return $name
}
"#;
    let arena = Bump::new();
    let result = compile_phpx(source, "/tmp/props.phpx", &arena);
    let diag_messages: Vec<String> = result
        .errors
        .iter()
        .map(|error| diagnostic_from_error("/tmp/props.phpx", source, error).message)
        .collect();
    assert!(
        diag_messages
            .iter()
            .any(|m| { m.contains("Destructured parameter") && m.contains("use interface") }),
        "messages={diag_messages:?}"
    );
}

#[test]
fn diagnostics_accept_interface_typed_destructured_props() {
    let source = r#"
interface NameProps { $name: string }
function FullName({ $name }: NameProps): string {
  return $name
}
"#;
    let arena = Bump::new();
    let result = compile_phpx(source, "/tmp/props_ok.phpx", &arena);
    let has_destructure_struct_error = result
        .errors
        .iter()
        .any(|error| error.message.contains("Destructured parameter"));
    assert!(
        !has_destructure_struct_error,
        "unexpected errors={:?}",
        result.errors
    );
}

#[test]
fn diagnostics_report_unknown_jsx_prop_with_suggestion() {
    let source = r#"
interface NameProps { $name: string; }
function FullName($props: NameProps): string {
  return $props.name;
}
$v = <FullName nam="Bob" />;
"#;
    let arena = Bump::new();
    let result = compile_phpx(source, "/tmp/props_typo.phpx", &arena);
    let messages: Vec<String> = result.errors.iter().map(|e| e.message.clone()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("Unknown prop 'nam'") && m.contains("did you mean 'name'")),
        "messages={messages:?}"
    );
}

#[test]
fn diagnostics_report_unknown_variable_with_suggestion() {
    let source = r#"
function fullName($name: string): string {
  return $nam;
}
"#;
    let arena = Bump::new();
    let result = compile_phpx(source, "/tmp/var_typo.phpx", &arena);
    let messages: Vec<String> = result.errors.iter().map(|e| e.message.clone()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("Unknown variable '$nam'") && m.contains("did you mean '$name'")),
        "messages={messages:?}"
    );
}

#[test]
fn diagnostics_report_missing_required_props_in_template_section() {
    let source = r#"---
interface NameProps {
  $name: string;
}
function FullName($props: NameProps): string {
  return $props.name;
}
---
<div>
  <FullName />
</div>
"#;
    let arena = Bump::new();
    let result = compile_phpx(source, "/tmp/template_missing_props.phpx", &arena);
    let missing = result
        .errors
        .iter()
        .find(|e| e.message.contains("Missing required prop 'name'"))
        .expect("missing required prop diagnostic");
    assert_eq!(missing.line, 10, "diagnostic={missing:?}");
    assert!(missing.column >= 3, "diagnostic={missing:?}");
    let messages: Vec<String> = result.errors.iter().map(|e| e.message.clone()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("Missing required prop 'name'")
                && m.contains("component 'FullName'")),
        "messages={messages:?}"
    );
}

#[test]
fn hover_shows_interface_shape() {
    let source = "interface NameProps { $name: string; }\nfunction FullName($props: NameProps): string { return $props.name; }\n";
    let file_path = "/tmp/hover_iface.phpx";
    let arena = Bump::new();
    let result = compile_phpx(source, file_path, &arena);
    let program = result.ast.expect("ast");
    let index = build_index(&program, source.as_bytes());
    let offset = source.find("NameProps").expect("offset");
    let hover = index.hover_at(offset).expect("hover");
    assert!(
        hover.contains("interface NameProps") && hover.contains("$name: string"),
        "hover={hover}"
    );
}

#[test]
fn index_infers_destructured_default_binding_type() {
    let source = r#"
function FullName({ age: $age = 18 }: Object<{ age: int }>): int {
  return $age;
}
"#;
    let arena = Bump::new();
    let result = compile_phpx(source, "/tmp/destructure_default.phpx", &arena);
    let program = result.ast.expect("ast");
    let index = build_index(&program, source.as_bytes());
    let function = index
        .functions
        .iter()
        .find(|f| f.name == "FullName")
        .expect("function");
    let has_int_binding = function
        .vars
        .iter()
        .any(|v| v.name == "$age" && v.ty.as_deref() == Some("int"));
    assert!(
        has_int_binding,
        "expected at least one `$age` binding inferred as int"
    );
}

#[test]
fn skips_unused_warning_when_unresolved_import_exists_at_same_span() {
    let warning = ValidationWarning {
        kind: modules_php::validation::ErrorKind::ImportError,
        line: 1,
        column: 10,
        message: "Unused import 'stat'.".to_string(),
        help_text: String::new(),
        suggestion: None,
        underline_length: 4,
        severity: Severity::Warning,
    };
    let mut unresolved = std::collections::HashSet::new();
    unresolved.insert((0, 9, 0, 13));
    assert!(should_skip_unused_import_warning(&warning, &unresolved));
}

#[test]
fn keeps_non_unused_or_non_overlapping_warnings() {
    let warning = ValidationWarning {
        kind: modules_php::validation::ErrorKind::ImportError,
        line: 1,
        column: 10,
        message: "Unused import 'stat'.".to_string(),
        help_text: String::new(),
        suggestion: None,
        underline_length: 4,
        severity: Severity::Warning,
    };
    let unresolved = std::collections::HashSet::new();
    assert!(!should_skip_unused_import_warning(&warning, &unresolved));
}

#[test]
fn skips_template_section_jsx_diagnostics() {
    let err = ValidationError {
        kind: modules_php::validation::ErrorKind::JsxError,
        line: 12,
        column: 5,
        message: "Mismatched closing tag".to_string(),
        help_text: "Fix JSX/template syntax in the template section.".to_string(),
        suggestion: None,
        underline_length: 4,
        severity: Severity::Error,
    };
    assert!(should_skip_template_html_diagnostic(&err));
}
