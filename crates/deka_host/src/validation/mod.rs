pub mod imports;
pub mod modules;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorKind {
    SyntaxError,
    UnexpectedToken,
    InvalidToken,
    TypeError,
    TypeMismatch,
    UnknownType,
    ImportError,
    ExportError,
    ModuleError,
    WasmError,
    NullNotAllowed,
    UndefinedNotAllowed,
    ExceptionNotAllowed,
    OopNotAllowed,
    NamespaceNotAllowed,
    JsxError,
    StructError,
    EnumError,
    PatternError,
    CypherError,
}

impl ErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::SyntaxError => "Syntax Error",
            ErrorKind::UnexpectedToken => "Unexpected Token",
            ErrorKind::InvalidToken => "Invalid Token",
            ErrorKind::TypeError => "Type Error",
            ErrorKind::TypeMismatch => "Type Mismatch",
            ErrorKind::UnknownType => "Unknown Type",
            ErrorKind::ImportError => "Import Error",
            ErrorKind::ExportError => "Export Error",
            ErrorKind::ModuleError => "Module Error",
            ErrorKind::WasmError => "WASM Error",
            ErrorKind::NullNotAllowed => "Null Not Allowed",
            ErrorKind::UndefinedNotAllowed => "Undefined Not Allowed",
            ErrorKind::ExceptionNotAllowed => "Exceptions Not Allowed",
            ErrorKind::OopNotAllowed => "OOP Not Allowed",
            ErrorKind::NamespaceNotAllowed => "Namespace Not Allowed",
            ErrorKind::JsxError => "JSX Error",
            ErrorKind::StructError => "Struct Error",
            ErrorKind::EnumError => "Enum Error",
            ErrorKind::PatternError => "Pattern Error",
            ErrorKind::CypherError => "Cypher Error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationError {
    pub kind: ErrorKind,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub help_text: String,
    pub suggestion: Option<String>,
    pub underline_length: usize,
    pub severity: Severity,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationWarning {
    pub kind: ErrorKind,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub help_text: String,
    pub suggestion: Option<String>,
    pub underline_length: usize,
    pub severity: Severity,
}

pub fn format_validation_error(source: &str, file_path: &str, error: &ValidationError) -> String {
    deka_validation::format_validation_error_with_suggestion(
        source,
        file_path,
        error.kind.as_str(),
        error.line,
        error.column,
        &error.message,
        &error.help_text,
        error.underline_length,
        severity_label(error.severity),
        docs_link_for_kind(error.kind),
        error.suggestion.clone(),
    )
}

pub fn format_validation_warning(
    source: &str,
    file_path: &str,
    warning: &ValidationWarning,
) -> String {
    deka_validation::format_validation_error_with_suggestion(
        source,
        file_path,
        warning.kind.as_str(),
        warning.line,
        warning.column,
        &warning.message,
        &warning.help_text,
        warning.underline_length,
        severity_label(warning.severity),
        docs_link_for_kind(warning.kind),
        warning.suggestion.clone(),
    )
}

pub fn format_multiple_errors(
    source: &str,
    file_path: &str,
    errors: &[ValidationError],
    warnings: &[ValidationWarning],
) -> String {
    let mut out = String::new();
    for error in errors {
        out.push_str(&format_validation_error(source, file_path, error));
    }
    for warning in warnings {
        out.push_str(&format_validation_warning(source, file_path, warning));
    }
    out
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn docs_link_for_kind(kind: ErrorKind) -> Option<String> {
    let path = match kind {
        ErrorKind::SyntaxError | ErrorKind::UnexpectedToken | ErrorKind::InvalidToken => {
            "docs/phpx/syntax"
        }
        ErrorKind::ImportError | ErrorKind::ExportError | ErrorKind::ModuleError => {
            "docs/phpx/modules"
        }
        ErrorKind::WasmError => "docs/phpx/wasm",
        ErrorKind::NullNotAllowed
        | ErrorKind::UndefinedNotAllowed
        | ErrorKind::ExceptionNotAllowed => "docs/phpx/strict",
        ErrorKind::OopNotAllowed => "docs/phpx/oop",
        ErrorKind::NamespaceNotAllowed => "docs/phpx/modules",
        ErrorKind::JsxError => "docs/phpx/jsx",
        ErrorKind::StructError => "docs/phpx/structs",
        ErrorKind::EnumError | ErrorKind::PatternError => "docs/phpx/enums",
        ErrorKind::TypeError | ErrorKind::TypeMismatch | ErrorKind::UnknownType => "docs/phpx/types",
        ErrorKind::CypherError => return None,
    };
    Some(path.to_string())
}
