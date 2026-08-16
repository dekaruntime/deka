use std::path::Path;

use php_rs::parser::ast::{Program, Severity as PhpSeverity};
use php_rs::phpx::typeck::{
    ExternalFunctionSig, TypeError as PhpTypeError, check_program_with_path,
    check_program_with_path_and_externals,
};

use super::{ErrorKind, Severity, ValidationError, ValidationWarning};

/// Type-check a program, splitting the diagnostics php-rs returns into
/// errors and warnings (deka#59).
///
/// `check_program_with_path`'s `Result` discriminant already tells us
/// whether the program checked out (`Ok`, diagnostics all `Warning`) or not
/// (`Err`, at least one `Error` present, possibly mixed with warnings) --
/// either way we still have to partition the returned `Vec<TypeError>` by
/// severity because `Err` may contain both kinds.
pub fn check_types(
    program: &Program,
    source: &str,
    file_path: Option<&str>,
) -> (Vec<ValidationError>, Vec<ValidationWarning>) {
    let path = file_path.filter(|path| !path.is_empty()).map(Path::new);
    let diagnostics = match check_program_with_path(program, source.as_bytes(), path) {
        Ok(diagnostics) | Err(diagnostics) => diagnostics,
    };
    partition_diagnostics(diagnostics, source)
}

pub fn check_types_with_externals(
    program: &Program,
    source: &str,
    file_path: Option<&str>,
    externals: &std::collections::HashMap<String, ExternalFunctionSig>,
) -> (Vec<ValidationError>, Vec<ValidationWarning>) {
    let path = file_path.filter(|path| !path.is_empty()).map(Path::new);
    let diagnostics = match check_program_with_path_and_externals(
        program,
        source.as_bytes(),
        path,
        externals,
    ) {
        Ok(diagnostics) | Err(diagnostics) => diagnostics,
    };
    partition_diagnostics(diagnostics, source)
}

fn partition_diagnostics(
    diagnostics: Vec<PhpTypeError>,
    source: &str,
) -> (Vec<ValidationError>, Vec<ValidationWarning>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for diagnostic in diagnostics {
        match diagnostic.severity {
            PhpSeverity::Error => errors.push(to_validation_error(diagnostic, source)),
            PhpSeverity::Warning => warnings.push(to_validation_warning(diagnostic, source)),
        }
    }
    (errors, warnings)
}

fn to_validation_error(error: PhpTypeError, source: &str) -> ValidationError {
    let (line, column, underline_length) = span_location(error.span, source);
    ValidationError {
        kind: ErrorKind::TypeError,
        line,
        column,
        message: error.message,
        help_text: "Fix the type mismatch or update the annotation.".to_string(),
        suggestion: None,
        underline_length,
        severity: Severity::Error,
    }
}

fn to_validation_warning(warning: PhpTypeError, source: &str) -> ValidationWarning {
    let (line, column, underline_length) = span_location(warning.span, source);
    ValidationWarning {
        kind: ErrorKind::TypeError,
        line,
        column,
        message: warning.message,
        help_text: "This still compiles; the message explains why it's flagged.".to_string(),
        suggestion: None,
        underline_length,
        severity: Severity::Warning,
    }
}

fn span_location(span: php_rs::parser::span::Span, source: &str) -> (usize, usize, usize) {
    if let Some(info) = span.line_info(source.as_bytes()) {
        let padding = std::cmp::min(info.line_text.len(), info.column.saturating_sub(1));
        let highlight_len = std::cmp::max(
            1,
            std::cmp::min(span.len(), info.line_text.len().saturating_sub(padding)),
        );
        (info.line, info.column, highlight_len)
    } else {
        (1, 1, 1)
    }
}
