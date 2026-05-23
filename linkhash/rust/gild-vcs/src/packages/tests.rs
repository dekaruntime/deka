use super::git::parse_ls_tree_line;
use super::manifest::declared_capabilities;
use super::snapshot::{parse_file_exports, parse_reexport_names};
use super::types::{ApiChangeKind, ApiSnapshot, ExportSignature};
use super::versioning::{classify_change, minimum_for_bump};
use semver::Version;
use std::collections::BTreeMap;

fn snap(items: &[(&str, &str)]) -> ApiSnapshot {
    let mut exports = BTreeMap::new();
    for (k, sig) in items {
        exports.insert(
            (*k).to_string(),
            ExportSignature {
                kind: "function".to_string(),
                signature: (*sig).to_string(),
                source: "src/index.phpx:1".to_string(),
                summary: None,
                description: None,
                examples: Vec::new(),
            },
        );
    }
    ApiSnapshot { exports }
}

#[test]
fn classify_patch_when_no_change() {
    let a = snap(&[("src/index::foo", "export function foo($x: int): int {")]);
    let b = snap(&[("src/index::foo", "export function foo($x: int): int {")]);
    let (kind, reasons, issues) = classify_change(&a, &b);
    assert_eq!(kind, ApiChangeKind::Patch);
    assert_eq!(reasons, vec!["no public API changes"]);
    assert!(issues.is_empty());
}

#[test]
fn classify_minor_when_added_export() {
    let a = snap(&[("src/index::foo", "export function foo($x: int): int {")]);
    let b = snap(&[
        ("src/index::foo", "export function foo($x: int): int {"),
        ("src/index::bar", "export function bar(): int {"),
    ]);
    let (kind, reasons, issues) = classify_change(&a, &b);
    assert_eq!(kind, ApiChangeKind::Minor);
    assert!(reasons.iter().any(|r| r.contains("added export")));
    assert!(issues.iter().any(|i| i.code == "API_ADDED_EXPORT"));
}

#[test]
fn classify_major_when_changed_or_removed() {
    let a = snap(&[
        ("src/index::foo", "export function foo($x: int): int {"),
        ("src/index::bar", "export function bar(): int {"),
    ]);
    let b = snap(&[("src/index::foo", "export function foo($x: string): int {")]);
    let (kind, reasons, issues) = classify_change(&a, &b);
    assert_eq!(kind, ApiChangeKind::Major);
    assert!(reasons.iter().any(|r| r.contains("changed export")));
    assert!(reasons.iter().any(|r| r.contains("removed export")));
    assert!(issues.iter().any(|i| i.code == "API_CHANGED_EXPORT"));
    assert!(issues.iter().any(|i| i.code == "API_REMOVED_EXPORT"));
}

#[test]
fn parses_reexport_aliases() {
    let names = parse_reexport_names("export { Foo as Bar, Baz } from './x.phpx';");
    assert_eq!(names, vec!["Bar".to_string(), "Baz".to_string()]);
}

#[test]
fn parses_multiline_reexport_exports() {
    let source = r#"
export {
  json_encode,
  json_decode,
} from 'encoding/json';
"#;
    let mut out = BTreeMap::new();
    parse_file_exports("json/index.phpx", source, &mut out);
    assert!(out.contains_key("json/index::json_encode"));
    assert!(out.contains_key("json/index::json_decode"));
}

#[test]
fn semver_minimum_for_bumps() {
    let v = Version::parse("1.2.3").unwrap();
    assert_eq!(
        minimum_for_bump(&v, ApiChangeKind::Patch).to_string(),
        "1.2.4"
    );
    assert_eq!(
        minimum_for_bump(&v, ApiChangeKind::Minor).to_string(),
        "1.3.0"
    );
    assert_eq!(
        minimum_for_bump(&v, ApiChangeKind::Major).to_string(),
        "2.0.0"
    );
}

#[test]
fn parses_declared_capabilities_from_manifest() {
    let manifest = serde_json::json!({
        "deka.security": {
            "allow": {
                "run": ["git"],
                "dynamic": true
            }
        }
    });
    let declared = declared_capabilities(Some(&manifest));
    assert!(declared.iter().any(|v| v == "run"));
    assert!(declared.iter().any(|v| v == "dynamic"));
}

#[test]
fn parses_doc_comment_above_export() {
    let source = r#"
/** Adds two integers.
 * More detail line.
 * @example sum(1, 2)
 */
export function sum($a: int, $b: int): int {
  return $a + $b;
}
"#;
    let mut out = BTreeMap::new();
    parse_file_exports("math.phpx", source, &mut out);
    let entry = out.get("math::sum").expect("export");
    assert_eq!(entry.summary.as_deref(), Some("Adds two integers."));
    assert_eq!(
        entry.description.as_deref(),
        Some("Adds two integers.\nMore detail line.")
    );
    assert_eq!(entry.examples, vec!["sum(1, 2)".to_string()]);
}

#[test]
fn parses_triple_slash_xml_description() {
    let source = r#"
/// docid: phpx/array/array()
/// <Function name="array">
///   <Description>
///     Creates an array from the given arguments.
///   </Description>
///   <Parameter name="$values" type="mixed" required="false" />
///   <ReturnType type="array" />
/// </Function>
export function array() {
  return [];
}
"#;
    let mut out = BTreeMap::new();
    parse_file_exports("array.phpx", source, &mut out);
    let entry = out.get("array::array").expect("export");
    assert_eq!(
        entry.summary.as_deref(),
        Some("Creates an array from the given arguments.")
    );
    assert_eq!(
        entry.description.as_deref(),
        Some("Creates an array from the given arguments.")
    );
    assert_eq!(entry.signature, "array($values mixed): array");
}

#[test]
fn parses_ls_tree_rows() {
    let row = "100644 blob abcdef1234567890 42\tsrc/main.phpx";
    let parsed = parse_ls_tree_line(row).expect("tree entry");
    assert_eq!(parsed.mode, "100644");
    assert_eq!(parsed.kind, "blob");
    assert_eq!(parsed.object, "abcdef1234567890");
    assert_eq!(parsed.size, Some(42));
    assert_eq!(parsed.path, "src/main.phpx");
}
