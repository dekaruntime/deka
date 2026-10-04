use super::*;
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
fn finds_whole_word_occurrences_only() {
    let src = b"foo food foo\nfoo_bar foo\n";
    let spans = find_word_occurrences(src, "foo");
    let ranges: Vec<(usize, usize)> = spans.into_iter().map(|s| (s.start, s.end)).collect();
    assert_eq!(ranges, vec![(0, 3), (9, 12), (21, 24)]);
}

#[test]
fn collects_module_rename_edits_across_workspace_files() {
    let dir = temp_dir("dekascript_lsp_module_rename");
    let file_a = dir.join("a.ds");
    let file_b = dir.join("b.ds");
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
    let dir = temp_dir("dekascript_lsp_symbol_rename");
    let file_a = dir.join("a.ds");
    let file_b = dir.join("b.ds");
    let src_a = "const foo = 1;\nconst food = 2;\n";
    let src_b = "function run(foo: number): number { return foo; }\n";
    fs::write(&file_a, src_a).expect("write a");
    fs::write(&file_b, src_b).expect("write b");

    let uri_a = Url::from_file_path(&file_a).expect("uri a");
    let edits =
        collect_symbol_rename_edits(std::slice::from_ref(&dir), &uri_a, src_a, "foo", "bar");
    let uri_b = Url::from_file_path(&file_b).expect("uri b");
    assert_eq!(edits.get(&uri_a).map(|v| v.len()), Some(1));
    assert_eq!(edits.get(&uri_b).map(|v| v.len()), Some(2));
}

#[test]
fn collects_references_across_workspace_files() {
    let dir = temp_dir("dekascript_lsp_refs");
    let file_a = dir.join("a.ds");
    let file_b = dir.join("b.ds");
    let src_a = "function run(user: string): string { return user; }\n";
    let src_b = "const user = 'sami';\n";
    fs::write(&file_a, src_a).expect("write a");
    fs::write(&file_b, src_b).expect("write b");

    let uri_a = Url::from_file_path(&file_a).expect("uri a");
    let refs = collect_reference_locations(std::slice::from_ref(&dir), &uri_a, src_a, "user");
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
