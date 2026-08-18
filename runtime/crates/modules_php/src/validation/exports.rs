use std::collections::{HashMap, HashSet};

use php_rs::parser::ast::{ClassKind, Program, Stmt};

use super::{ErrorKind, Severity, ValidationError};
use crate::validation::imports::{
    consume_comment_line, find_column, frontmatter_bounds, is_ident, parse_import_line,
    parse_quoted_string, strip_php_tags_inline,
};

#[derive(Debug, Clone)]
pub(crate) struct ExportSpec {
    pub(crate) name: String,
    line: usize,
    column: usize,
    is_reexport: bool,
}

pub fn validate_exports(
    source: &str,
    file_path: &str,
    program: &Program,
    is_ds: bool,
) -> Vec<ValidationError> {
    let lines: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_bounds(&lines);
    let scan_end = bounds.map(|(_, end)| end).unwrap_or(lines.len());
    let is_template = bounds
        .map(|(_, end)| {
            lines
                .iter()
                .skip(end + 1)
                .any(|line| !line.trim().is_empty())
        })
        .unwrap_or(false);

    let import_locals = collect_import_locals(&lines, bounds, file_path);

    let mut errors = Vec::new();
    let mut exports = Vec::new();
    let mut in_block_comment = false;

    for (idx, line) in lines.iter().enumerate().take(scan_end) {
        if let Some((start, end)) = bounds {
            if idx == start || idx == end {
                continue;
            }
        }

        let clean = strip_php_tags_inline(line);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            continue;
        }
        if consume_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }

        if trimmed.starts_with("export function")
            || trimmed.starts_with("export async function")
            || (is_ds
                && (trimmed.starts_with("export fn") || trimmed.starts_with("export async fn")))
        {
            if is_template {
                errors.push(export_error(
                    idx + 1,
                    find_column(line, "export"),
                    trimmed.len(),
                    "Explicit exports are not allowed in template files.".to_string(),
                    "Template components are auto-exported. Remove the export keyword.",
                ));
                continue;
            }
            match parse_export_function(trimmed, line, idx + 1, file_path) {
                Ok(spec) => exports.push(spec),
                Err(err) => errors.push(err),
            }
            continue;
        }

        if trimmed.starts_with("export const ") {
            if is_ds {
                match parse_export_const(trimmed, line, idx + 1, file_path) {
                    Ok(spec) => exports.push(spec),
                    Err(err) => errors.push(err),
                }
            } else {
                errors.push(export_error_with_suggestion(
                    idx + 1,
                    find_column(line, "export"),
                    trimmed.len(),
                    format!("Unsupported export syntax in {}.", file_path),
                    "Use `export function name(...)` or `export { name }` in PHPX.",
                    Some("export { name };"),
                ));
            }
            continue;
        }

        if trimmed.starts_with("export {") {
            if is_template {
                errors.push(export_error(
                    idx + 1,
                    find_column(line, "export"),
                    trimmed.len(),
                    "Explicit exports are not allowed in template files.".to_string(),
                    "Template components are auto-exported. Remove the export statement.",
                ));
                continue;
            }
            match parse_export_list_line(trimmed, line, idx + 1, file_path) {
                Ok(mut specs) => exports.append(&mut specs),
                Err(err) => errors.push(err),
            }
            continue;
        }

        if trimmed.starts_with("export ") {
            errors.push(export_error_with_suggestion(
                idx + 1,
                find_column(line, "export"),
                trimmed.len(),
                format!("Unsupported export syntax in {}.", file_path),
                "Use `export function name(...)`, `export const name = ...`, or `export { name }`.",
                Some("export function name() { }"),
            ));
        }
    }

    let exportables = collect_exportables(program, source);
    let mut seen = HashMap::new();
    for spec in &exports {
        if let Some((line, column)) = seen.get(&spec.name) {
            errors.push(export_error(
                spec.line,
                spec.column,
                spec.name.len().max(1),
                format!(
                    "Duplicate export '{}'. First declared at line {}, column {}.",
                    spec.name, line, column
                ),
                "Remove the duplicate export.",
            ));
        } else {
            seen.insert(spec.name.clone(), (spec.line, spec.column));
        }
    }

    for spec in exports {
        if spec.is_reexport {
            continue;
        }
        if exportables.contains(&spec.name) {
            continue;
        }
        if import_locals.contains(&spec.name) {
            continue;
        }
        errors.push(export_error(
            spec.line,
            spec.column,
            spec.name.len().max(1),
            format!("Export '{}' is not defined in {}.", spec.name, file_path),
            "Define the function, const, struct, or type before exporting.",
        ));
    }

    errors
}

fn parse_export_const(line: &str, raw_line: &str, line_number: usize, file_path: &str) -> Result<ExportSpec, ValidationError> {
    let rest = line.trim_start_matches("export const ").trim_start();
    let name = rest.split(|ch: char| ch == '=' || ch.is_whitespace()).next().filter(|name| is_ident(name)).ok_or_else(|| export_error(line_number, find_column(raw_line, "const"), line.trim().len(), format!("Expected a const name after `export const` in {}.", file_path), "Write `export const name = value;"))?;
    Ok(ExportSpec { name: name.to_string(), line: line_number, column: find_column(raw_line, name), is_reexport: false })
}

fn collect_import_locals(
    lines: &[&str],
    bounds: Option<(usize, usize)>,
    file_path: &str,
) -> HashSet<String> {
    let mut locals = HashSet::new();
    let scan_end = bounds.map(|(_, end)| end).unwrap_or(lines.len());
    let mut in_block_comment = false;
    for (idx, line) in lines.iter().enumerate().take(scan_end) {
        if let Some((start, end)) = bounds {
            if idx == start || idx == end {
                continue;
            }
        }
        let clean = strip_php_tags_inline(line);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            continue;
        }
        if consume_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }
        if trimmed.starts_with("import ") {
            if let Ok(specs) = parse_import_line(trimmed, line, idx + 1, file_path) {
                for spec in specs {
                    locals.insert(spec.local);
                }
            }
        }
    }
    locals
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

fn collect_exportables(program: &Program, source: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in program.statements {
        match stmt {
            Stmt::Function { name, .. } => {
                if let Ok(text) = std::str::from_utf8(name.text(source.as_bytes())) {
                    names.insert(text.to_string());
                }
            }
            Stmt::Const { consts, .. } => {
                for constant in *consts {
                    if let Ok(text) = std::str::from_utf8(constant.name.text(source.as_bytes())) {
                        names.insert(text.to_string());
                    }
                }
            }
            Stmt::TypeAlias { name, .. } => {
                if let Ok(text) = std::str::from_utf8(name.text(source.as_bytes())) {
                    names.insert(text.to_string());
                }
            }
            Stmt::Class { kind, name, .. } => {
                if matches!(kind, ClassKind::Struct) {
                    if let Ok(text) = std::str::from_utf8(name.text(source.as_bytes())) {
                        names.insert(text.to_string());
                    }
                }
            }
            Stmt::Enum { name, .. } => {
                if let Ok(text) = std::str::from_utf8(name.text(source.as_bytes())) {
                    names.insert(text.to_string());
                }
            }
            _ => {}
        }
    }
    names
}

fn export_error(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
) -> ValidationError {
    export_error_with_suggestion(line, column, underline_length, message, help_text, None)
}

fn export_error_with_suggestion(
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
