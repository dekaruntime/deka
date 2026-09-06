//! Transport-neutral DekaScript analysis types.
//!
//! Compiler diagnostics come from `dsc lsp`. This crate no longer links
//! the in-process compiler.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisContext {
    /// URI or path passed through to the compiler for source context.
    pub uri_or_path: String,
}

impl AnalysisContext {
    pub fn new(uri_or_path: impl Into<String>) -> Self {
        Self {
            uri_or_path: uri_or_path.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisSeverity {
    Error,
    Warning,
    Information,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisPosition {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based UTF-16 code-unit offset within the line.
    pub character: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisRange {
    pub start: AnalysisPosition,
    pub end: AnalysisPosition,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisDiagnostic {
    pub range: AnalysisRange,
    pub severity: AnalysisSeverity,
    pub code: String,
    pub message: String,
}

/// Compiler diagnostics are produced by `dsc lsp`. This host-side analyze
/// path is a no-op so the LSP crate does not link the in-process compiler.
pub fn analyze(_source: &str, _context: &AnalysisContext) -> Vec<AnalysisDiagnostic> {
    Vec::new()
}

pub fn is_dekascript_context(context: &AnalysisContext) -> bool {
    let path = context
        .uri_or_path
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ds"))
}

#[cfg(test)]
fn source_range(
    source: &str,
    line: usize,
    column: usize,
    underline_length: usize,
) -> AnalysisRange {
    let lines: Vec<&str> = source.split('\n').collect();
    let line_index = line.saturating_sub(1).min(lines.len().saturating_sub(1));
    let current_line = lines.get(line_index).copied().unwrap_or_default();
    let start_byte = floor_char_boundary(current_line, column.saturating_sub(1));
    let end_byte = floor_char_boundary(
        current_line,
        start_byte
            .saturating_add(underline_length.max(1))
            .min(current_line.len()),
    );
    AnalysisRange {
        start: AnalysisPosition {
            line: line_index as u32,
            character: utf16_offset_at_byte(current_line, start_byte),
        },
        end: AnalysisPosition {
            line: line_index as u32,
            character: utf16_offset_at_byte(current_line, end_byte),
        },
    }
}

#[cfg(test)]
fn floor_char_boundary(line: &str, byte_offset: usize) -> usize {
    let mut byte_offset = byte_offset.min(line.len());
    while byte_offset > 0 && !line.is_char_boundary(byte_offset) {
        byte_offset -= 1;
    }
    byte_offset
}

#[cfg(test)]
fn utf16_offset_at_byte(line: &str, byte_offset: usize) -> u32 {
    line[..floor_char_boundary(line, byte_offset)]
        .encode_utf16()
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_is_a_no_op_without_in_process_compiler() {
        let diagnostics = analyze(
            "const = ;\n",
            &AnalysisContext::new("file:///workspace/main.ds"),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn leaves_resolver_owned_package_imports_unflagged() {
        let source = r#"
            import { Widget } from "@acme/widgets"
            import { helper } from "@user/helpers"
        "#;
        let diagnostics = analyze(source, &AnalysisContext::new("file:///workspace/main.ds"));
        assert!(
            diagnostics.is_empty(),
            "resolver-owned package imports must not be rejected by single-file LSP analysis: {diagnostics:?}"
        );
    }

    #[test]
    fn match_exhaustiveness_is_owned_by_dsc_lsp() {
        let source = r#"
            enum Color { Red, Green, Blue }

            fn label(c: Color) string {
                return match (c) {
                    Color.Red => "red",
                    Color.Green => "green",
                }
            }
        "#;
        let diagnostics = analyze(source, &AnalysisContext::new("file:///workspace/main.ds"));
        assert!(
            diagnostics.is_empty(),
            "in-process analyze must not compile; dsc lsp owns typeck: {diagnostics:?}"
        );
    }

    #[test]
    fn ignores_legacy_source_contexts() {
        assert!(analyze("const = ;", &AnalysisContext::new("legacy.phpx")).is_empty());
        assert!(analyze("const = ;", &AnalysisContext::new("legacy.php")).is_empty());
    }

    #[test]
    fn ranges_are_clamped_to_source_utf16_boundaries() {
        let range = source_range("éx\n", 99, 99, 99);
        assert_eq!(
            range.start,
            AnalysisPosition {
                line: 1,
                character: 0
            }
        );
        assert_eq!(
            range.end,
            AnalysisPosition {
                line: 1,
                character: 0
            }
        );
        let range = source_range("éx", 1, 3, 1);
        assert_eq!(
            range.start,
            AnalysisPosition {
                line: 0,
                character: 1
            }
        );
        assert_eq!(
            range.end,
            AnalysisPosition {
                line: 0,
                character: 2
            }
        );
    }

    #[test]
    fn range_after_non_ascii_uses_utf16() {
        let range = source_range("const label = 'é'; const = ;\n", 1, 26, 1);
        assert_eq!(
            range.start.character + 1,
            range.end.character,
            "{range:?}"
        );
    }
}
