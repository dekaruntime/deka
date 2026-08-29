use std::collections::{HashMap, HashSet};

use php_rs::parser::ast::{ClassKind, ExportItem, Program, Stmt};

use super::{ErrorKind, Severity, ValidationError};
use crate::validation::export_parsers::{
    ExportSpec, export_error, export_error_with_suggestion, parse_export_function,
    parse_export_list_line,
};
use crate::validation::imports::{
    consume_comment_line, find_column, frontmatter_bounds, is_ident, parse_import_line,
    strip_php_tags_inline,
};

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
                    "Use `export function name(...)` or `export { name }` in DekaScript.",
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

fn parse_export_const(
    line: &str,
    raw_line: &str,
    line_number: usize,
    file_path: &str,
) -> Result<ExportSpec, ValidationError> {
    let rest = line.trim_start_matches("export const ").trim_start();
    let name = rest.split(|ch: char| ch == '=' || ch.is_whitespace()).next().filter(|name| is_ident(name)).ok_or_else(|| export_error(line_number, find_column(raw_line, "const"), line.trim().len(), format!("Expected a const name after `export const` in {}.", file_path), "Write `export const name = value;`"))?;
    Ok(ExportSpec {
        name: name.to_string(),
        line: line_number,
        column: find_column(raw_line, name),
        is_reexport: false,
    })
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
            Stmt::Export { item, .. } => {
                if let ExportItem::Decl(decl) = item {
                    match decl {
                        Stmt::Function { name, .. } => {
                            if let Ok(text) = std::str::from_utf8(name.text(source.as_bytes())) {
                                names.insert(text.to_string());
                            }
                        }
                        Stmt::Const { consts, .. } => {
                            for constant in *consts {
                                if let Ok(text) =
                                    std::str::from_utf8(constant.name.text(source.as_bytes()))
                                {
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
                                if let Ok(text) =
                                    std::str::from_utf8(name.text(source.as_bytes()))
                                {
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
            }
            _ => {}
        }
    }
    names
}
