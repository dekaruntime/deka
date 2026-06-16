use super::*;

pub(crate) fn diagnostic_from_error(
    file_path: &str,
    source: &str,
    error: &ValidationError,
) -> Diagnostic {
    let rendered = deka_validation::format_validation_error_with_suggestion(
        source,
        file_path,
        error.kind.as_str(),
        error.line,
        error.column,
        &error.message,
        &error.help_text,
        error.underline_length,
        severity_label(error.severity),
        None,
        error.suggestion.clone(),
    );
    Diagnostic {
        range: diagnostic_range(error.line, error.column, error.underline_length),
        severity: Some(severity_to_lsp(error.severity)),
        code: Some(tower_lsp::lsp_types::NumberOrString::String(
            error.kind.as_str().to_string(),
        )),
        source: Some("phpx".to_string()),
        message: strip_ansi_codes(&rendered),
        ..Diagnostic::default()
    }
}

pub(crate) fn diagnostic_from_warning(
    file_path: &str,
    source: &str,
    warning: &ValidationWarning,
) -> Diagnostic {
    let rendered = deka_validation::format_validation_error_with_suggestion(
        source,
        file_path,
        warning.kind.as_str(),
        warning.line,
        warning.column,
        &warning.message,
        &warning.help_text,
        warning.underline_length,
        severity_label(warning.severity),
        None,
        warning.suggestion.clone(),
    );
    Diagnostic {
        range: diagnostic_range(warning.line, warning.column, warning.underline_length),
        severity: Some(severity_to_lsp(warning.severity)),
        code: Some(tower_lsp::lsp_types::NumberOrString::String(
            warning.kind.as_str().to_string(),
        )),
        source: Some("phpx".to_string()),
        message: strip_ansi_codes(&rendered),
        ..Diagnostic::default()
    }
}

pub(crate) fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

pub(crate) fn strip_ansi_codes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() {
                let b = bytes[i];
                if (b as char).is_ascii_alphabetic() {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }

    let bytes = out.as_bytes();
    let mut cleaned = String::with_capacity(out.len());
    let mut j = 0usize;
    while j < bytes.len() {
        if bytes[j] == b'[' {
            let mut k = j + 1;
            let mut saw_digit = false;
            while k < bytes.len() && (bytes[k].is_ascii_digit() || bytes[k] == b';') {
                saw_digit = true;
                k += 1;
            }
            if saw_digit && k < bytes.len() && bytes[k] == b'm' {
                j = k + 1;
                continue;
            }
        }
        cleaned.push(bytes[j] as char);
        j += 1;
    }
    cleaned
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
            source: Some("phpx".to_string()),
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
                source: Some("phpx".to_string()),
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
            suggestion: "Run with `phpx.target = server` or move database access behind a server endpoint.",
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
