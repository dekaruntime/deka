use super::{ErrorKind, Severity, ValidationError};
use crate::validation::imports::{find_column, is_ident, parse_quoted_string};

#[derive(Debug, Clone)]
pub(crate) struct ExportSpec {
    pub(crate) name: String,
    pub(crate) line: usize,
    pub(crate) column: usize,
    #[allow(dead_code)]
    pub(crate) is_reexport: bool,
}

pub(crate) fn parse_export_function(
    line: &str,
    raw_line: &str,
    line_number: usize,
    file_path: &str,
) -> Result<ExportSpec, ValidationError> {
    let mut rest = line.trim_start().strip_prefix("export").unwrap_or(line).trim_start();
    if let Some(tail) = rest.strip_prefix("async") {
        rest = tail.trim_start();
    }
    rest = rest
        .strip_prefix("function")
        .or_else(|| rest.strip_prefix("fn"))
        .ok_or_else(|| {
            export_error_with_suggestion(
                line_number,
                find_column(raw_line, "export"),
                line.trim().len(),
                format!("Invalid export syntax in {}.", file_path),
                "Use `export function name(...)` or `export async function name(...)`. In DekaScript, use `export fn name(...)`.",
                Some("export function name() { }"),
            )
        })?
        .trim_start();

    let rest = if let Some(stripped) = rest.strip_prefix('&') {
        stripped.trim_start()
    } else {
        rest
    };

    let mut name = String::new();
    for ch in rest.chars() {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            name.push(ch);
        } else {
            break;
        }
    }

    if name.is_empty() || !is_ident(&name) {
        return Err(export_error_with_suggestion(
            line_number,
            find_column(raw_line, "export"),
            line.trim().len(),
            format!("Invalid export function name in {}.", file_path),
            "Function name must be a valid identifier.",
            Some("export function name() { }"),
        ));
    }

    Ok(ExportSpec {
        name,
        line: line_number,
        column: find_column(raw_line, "export"),
        is_reexport: false,
    })
}

pub(crate) fn parse_export_list_line(
    line: &str,
    raw_line: &str,
    line_number: usize,
    file_path: &str,
) -> Result<Vec<ExportSpec>, ValidationError> {
    let rest = line
        .trim_start()
        .strip_prefix("export")
        .unwrap_or(line)
        .trim_start();

    let rest = rest.strip_prefix('{').ok_or_else(|| {
        export_error_with_suggestion(
            line_number,
            find_column(raw_line, "export"),
            line.trim().len(),
            format!("Invalid export syntax in {}.", file_path),
            "Expected '{' after export.",
            Some("export { name };"),
        )
    })?;

    let close_idx = rest.find('}').ok_or_else(|| {
        export_error_with_suggestion(
            line_number,
            find_column(raw_line, "export"),
            line.trim().len(),
            format!("Invalid export syntax in {}.", file_path),
            "Missing closing '}' in export list.",
            Some("export { name };"),
        )
    })?;

    let specifiers = &rest[..close_idx];
    let mut after = rest[close_idx + 1..].trim_start();

    let mut is_reexport = false;
    if let Some(after_from) = after.strip_prefix("from") {
        is_reexport = true;
        after = after_from.trim_start();
        let (from, after_from) = parse_quoted_string(after).ok_or_else(|| {
            export_error(
                line_number,
                find_column(raw_line, "from"),
                line.trim().len(),
                format!("Invalid export syntax in {}.", file_path),
                "Module specifier must be quoted: from 'module'.",
            )
        })?;
        if from.contains("../") || from.contains("..\\") {
            return Err(export_error(
                line_number,
                find_column(raw_line, &from),
                from.len().max(1),
                format!(
                    "Relative module paths using '..' are not supported ('{}').",
                    from
                ),
                "Use a module name from php_modules/ instead of relative paths.",
            ));
        }
        after = after_from.trim_start();
    }

    let after = after.trim_start_matches(';').trim();
    if !after.is_empty() {
        return Err(export_error(
            line_number,
            find_column(raw_line, after),
            after.len().max(1),
            format!("Invalid export syntax in {}.", file_path),
            "Unexpected tokens after export statement.",
        ));
    }

    let spec_list: Vec<&str> = specifiers
        .split(',')
        .map(|spec| spec.trim())
        .filter(|spec| !spec.is_empty())
        .collect();

    if spec_list.is_empty() {
        return Err(export_error(
            line_number,
            find_column(raw_line, "export"),
            line.trim().len(),
            format!("Empty export list in {}.", file_path),
            "Add at least one export specifier.",
        ));
    }

    let mut exports = Vec::new();
    for spec in spec_list {
        let parts: Vec<&str> = spec.split_whitespace().collect();
        let (imported, local) = match parts.as_slice() {
            [name] => (*name, *name),
            [name, as_kw, alias] if *as_kw == "as" => (*name, *alias),
            _ => {
                return Err(export_error(
                    line_number,
                    find_column(raw_line, spec),
                    spec.len().max(1),
                    format!("Invalid export specifier '{}' in {}.", spec, file_path),
                    "Use `name` or `name as alias` inside the export list.",
                ));
            }
        };

        if !is_ident(imported) || !is_ident(local) {
            return Err(export_error(
                line_number,
                find_column(raw_line, spec),
                spec.len().max(1),
                format!("Invalid export specifier '{}' in {}.", spec, file_path),
                "Export names must be valid identifiers.",
            ));
        }

        if !is_reexport && imported != local && local != "default" {
            return Err(export_error(
                line_number,
                find_column(raw_line, spec),
                spec.len().max(1),
                format!("Unsupported export alias '{}' in {}.", spec, file_path),
                "Aliases are only supported with `export { name } from 'module'` or `export { name as default }`.",
            ));
        }

        exports.push(ExportSpec {
            name: local.to_string(),
            line: line_number,
            column: find_column(raw_line, local),
            is_reexport,
        });
    }

    Ok(exports)
}

pub(crate) fn export_error(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
) -> ValidationError {
    export_error_with_suggestion(line, column, underline_length, message, help_text, None)
}

pub(crate) fn export_error_with_suggestion(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
    suggestion: Option<&str>,
) -> ValidationError {
    ValidationError {
        kind: ErrorKind::ExportError,
        line,
        column,
        message,
        help_text: help_text.to_string(),
        suggestion: suggestion.map(|value| value.to_string()),
        underline_length: underline_length.max(1),
        severity: Severity::Error,
    }
}
