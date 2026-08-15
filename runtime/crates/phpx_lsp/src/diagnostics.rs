use super::*;

pub(crate) fn diagnostic_from_error(
    _file_path: &str,
    _source: &str,
    error: &ValidationError,
) -> Diagnostic {
    Diagnostic {
        range: diagnostic_range(error.line, error.column, error.underline_length),
        severity: Some(severity_to_lsp(error.severity)),
        code: Some(tower_lsp::lsp_types::NumberOrString::String(
            error.kind.as_str().to_string(),
        )),
        source: Some(LANGUAGE_ID.to_string()),
        message: plain_message(
            &error.message,
            &error.help_text,
            error.suggestion.as_deref(),
        ),
        ..Diagnostic::default()
    }
}

pub(crate) fn diagnostic_from_warning(
    _file_path: &str,
    _source: &str,
    warning: &ValidationWarning,
) -> Diagnostic {
    Diagnostic {
        range: diagnostic_range(warning.line, warning.column, warning.underline_length),
        severity: Some(severity_to_lsp(warning.severity)),
        code: Some(tower_lsp::lsp_types::NumberOrString::String(
            warning.kind.as_str().to_string(),
        )),
        source: Some(LANGUAGE_ID.to_string()),
        message: plain_message(
            &warning.message,
            &warning.help_text,
            warning.suggestion.as_deref(),
        ),
        ..Diagnostic::default()
    }
}

pub(crate) fn plain_message(message: &str, help_text: &str, suggestion: Option<&str>) -> String {
    let mut parts = vec![message.trim()];
    let help_text = help_text.trim();
    if !help_text.is_empty() {
        parts.push(help_text);
    }
    if let Some(suggestion) = suggestion.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(suggestion);
    }
    parts.join("\n")
}

pub(crate) fn severity_to_lsp(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
    }
}

pub(crate) fn diagnostic_range(line: usize, column: usize, underline_length: usize) -> Range {
    let line = line.saturating_sub(1) as u32;
    let start_char = column.saturating_sub(1) as u32;
    let end_char = start_char + underline_length.max(1) as u32;
    Range {
        start: Position {
            line,
            character: start_char,
        },
        end: Position {
            line,
            character: end_char,
        },
    }
}

pub(crate) fn should_skip_unused_import_warning(
    warning: &ValidationWarning,
    unresolved_ranges: &std::collections::HashSet<(u32, u32, u32, u32)>,
) -> bool {
    if !warning.message.contains("Unused import") {
        return false;
    }
    let range = diagnostic_range(warning.line, warning.column, warning.underline_length);
    let key = (
        range.start.line,
        range.start.character,
        range.end.line,
        range.end.character,
    );
    unresolved_ranges.contains(&key)
}
pub(crate) fn unresolved_import_diagnostics(
    source: &str,
    file_path: &str,
    workspace_roots: &[PathBuf],
) -> Vec<Diagnostic> {
    let root = match find_php_modules_root(Path::new(file_path), workspace_roots) {
        Some(root) => root,
        None => return Vec::new(),
    };

    let imports = parse_imports(source);
    if imports.is_empty() {
        return Vec::new();
    }

    let line_index = LineIndex::new(source);
    let mut module_cache: HashMap<(String, bool), Option<Vec<ExportInfo>>> = HashMap::new();
    let mut diagnostics = Vec::new();

    for import in imports {
        if import.imported == "default" {
            continue;
        }

        let key = (import.from.clone(), import.is_wasm);
        let exports = module_cache
            .entry(key)
            .or_insert_with(|| module_exports(&root, &import.from, import.is_wasm));
        let Some(exports) = exports else {
            continue;
        };

        let found = exports.iter().any(|export| export.name == import.imported);
        if found {
            continue;
        }

        diagnostics.push(Diagnostic {
            range: span_to_range(import.span, &line_index),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(tower_lsp::lsp_types::NumberOrString::String(
                "Import Error".to_string(),
            )),
            source: Some(LANGUAGE_ID.to_string()),
            message: format!(
                "Import Error: Module '{}' has no export named '{}'.",
                import.from, import.imported
            ),
            ..Diagnostic::default()
        });
    }

    diagnostics
}

pub(crate) fn target_capability_diagnostics(
    source: &str,
    target_mode: TargetMode,
) -> Vec<Diagnostic> {
    if target_mode == TargetMode::Server {
        return Vec::new();
    }

    let imports = parse_imports(source);
    if imports.is_empty() {
        return Vec::new();
    }

    let line_index = LineIndex::new(source);
    let mut diagnostics = Vec::new();
    for import in imports {
        if let Some(block) = adwa_capability_block(&import.from) {
            diagnostics.push(Diagnostic {
                range: span_to_range(import.module_span.unwrap_or(import.span), &line_index),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(
                    "Target Capability Error".to_string(),
                )),
                source: Some(LANGUAGE_ID.to_string()),
                message: format!(
                    "Target Capability Error: Module '{}' is unavailable for target 'adwa' ({}).\nhelp: {}",
                    import.from, block.reason, block.suggestion
                ),
                ..Diagnostic::default()
            });
        }
    }
    diagnostics
}

pub(crate) struct CapabilityBlock {
    reason: &'static str,
    suggestion: &'static str,
}

pub(crate) fn adwa_capability_block(module_spec: &str) -> Option<CapabilityBlock> {
    if module_spec == "db"
        || module_spec.starts_with("db/")
        || module_spec == "postgres"
        || module_spec.starts_with("postgres/")
        || module_spec == "mysql"
        || module_spec.starts_with("mysql/")
        || module_spec == "sqlite"
        || module_spec.starts_with("sqlite/")
    {
        return Some(CapabilityBlock {
            reason: "database host capability is disabled",
            suggestion: "Run with `dekascript.target = server` or move database access behind a server endpoint.",
        });
    }
    if module_spec == "process"
        || module_spec.starts_with("process/")
        || module_spec == "env"
        || module_spec.starts_with("env/")
    {
        return Some(CapabilityBlock {
            reason: "process/env host capability is disabled",
            suggestion: "Inject values through app config/context instead of reading process/env in `adwa`.",
        });
    }
    None
}
