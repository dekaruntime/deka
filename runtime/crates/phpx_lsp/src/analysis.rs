//! Transport-neutral DekaScript compiler analysis.
//!
//! The native LSP and a future WASM worker can consume these plain data types
//! without depending on a protocol transport.

use bumpalo::Bump;
use modules_php::compiler_api::compile_deka;
use modules_php::validation::{Severity as CompilerSeverity, ValidationError};
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

/// Analyzes DekaScript via `compile_deka` with no filesystem, async-runtime, or
/// transport dependency. Ranges are clamped to valid UTF-16 positions in source.
pub fn analyze(source: &str, context: &AnalysisContext) -> Vec<AnalysisDiagnostic> {
    if !is_dekascript_context(context) {
        return Vec::new();
    }
    let arena = Bump::new();
    let result = compile_deka(source, &context.uri_or_path, &arena);
    result
        .errors
        .into_iter()
        .filter(|error| !should_skip_template_html_diagnostic(error))
        .map(|error| AnalysisDiagnostic {
            range: source_range(source, error.line, error.column, error.underline_length),
            severity: severity(error.severity),
            code: error.kind.as_str().to_string(),
            message: plain_message(
                &error.message,
                &error.help_text,
                error.suggestion.as_deref(),
            ),
        })
        .chain(
            result
                .warnings
                .into_iter()
                .map(|warning| AnalysisDiagnostic {
                    range: source_range(
                        source,
                        warning.line,
                        warning.column,
                        warning.underline_length,
                    ),
                    severity: severity(warning.severity),
                    code: warning.kind.as_str().to_string(),
                    message: plain_message(
                        &warning.message,
                        &warning.help_text,
                        warning.suggestion.as_deref(),
                    ),
                }),
        )
        .collect()
}

fn is_dekascript_context(context: &AnalysisContext) -> bool {
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

fn should_skip_template_html_diagnostic(error: &ValidationError) -> bool {
    error
        .help_text
        .contains("Fix JSX/template syntax in the template section.")
}

fn severity(severity: CompilerSeverity) -> AnalysisSeverity {
    match severity {
        CompilerSeverity::Error => AnalysisSeverity::Error,
        CompilerSeverity::Warning => AnalysisSeverity::Warning,
        CompilerSeverity::Info => AnalysisSeverity::Information,
    }
}

fn plain_message(message: &str, help_text: &str, suggestion: Option<&str>) -> String {
    let mut parts = vec![message.trim()];
    if !help_text.trim().is_empty() {
        parts.push(help_text.trim());
    }
    if let Some(suggestion) = suggestion.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(suggestion);
    }
    parts.join("\n")
}

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

fn floor_char_boundary(line: &str, byte_offset: usize) -> usize {
    let mut byte_offset = byte_offset.min(line.len());
    while byte_offset > 0 && !line.is_char_boundary(byte_offset) {
        byte_offset -= 1;
    }
    byte_offset
}

fn utf16_offset_at_byte(line: &str, byte_offset: usize) -> u32 {
    line[..floor_char_boundary(line, byte_offset)]
        .encode_utf16()
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzes_dekascript_through_compile_deka() {
        let diagnostics = analyze(
            "const = ;\n",
            &AnalysisContext::new("file:///workspace/main.ds"),
        );
        assert!(!diagnostics.is_empty(), "expected compiler diagnostics");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.code.is_empty())
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
    fn compiler_diagnostic_after_non_ascii_uses_utf16_range() {
        let source = "const label = 'é'; const = ;\n";
        let diagnostics = analyze(source, &AnalysisContext::new("file:///workspace/main.ds"));
        let diagnostic = diagnostics.first().expect("compiler diagnostic");
        assert_eq!(
            diagnostic.range,
            AnalysisRange {
                start: AnalysisPosition {
                    line: 0,
                    character: 25,
                },
                end: AnalysisPosition {
                    line: 0,
                    character: 26,
                },
            }
        );
    }
}
