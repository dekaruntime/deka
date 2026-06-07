use std::collections::{HashMap, HashSet};

use super::{ErrorKind, Severity, ValidationError, ValidationWarning};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportKind {
    Phpx,
    Wasm,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportSpec {
    #[allow(dead_code)]
    pub(crate) imported: String,
    pub(crate) local: String,
    pub(crate) from: String,
    #[allow(dead_code)]
    pub(crate) kind: ImportKind,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) line_text: String,
}

pub fn validate_imports(
    source: &str,
    file_path: &str,
) -> (Vec<ValidationError>, Vec<ValidationWarning>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_bounds(&lines);
    let scan_end = bounds.map(|(_, end)| end).unwrap_or(lines.len());

    let mut import_lines = HashSet::new();
    let mut import_specs = Vec::new();
    let mut saw_code = false;
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
            if saw_code {
                errors.push(import_error(
                    idx + 1,
                    find_column(line, "import"),
                    trimmed.len(),
                    format!("Import must appear before other code in {}.", file_path),
                    "Move import statements to the top of the file.",
                ));
                continue;
            }

            match parse_import_line(trimmed, line, idx + 1, file_path) {
                Ok(mut specs) => {
                    import_lines.insert(idx);
                    import_specs.append(&mut specs);
                }
                Err(err) => {
                    errors.push(err);
                }
            }
            continue;
        }

        saw_code = true;
    }

    let mut seen_locals: HashMap<String, (usize, usize)> = HashMap::new();
    for spec in &import_specs {
        if let Some((line, column)) = seen_locals.get(&spec.local) {
            errors.push(import_error(
                spec.line,
                spec.column,
                spec.local.len().max(1),
                format!(
                    "Duplicate import '{}'. First declared at line {}, column {}.",
                    spec.local, line, column
                ),
                "Remove the duplicate import or rename the local binding.",
            ));
        } else {
            seen_locals.insert(spec.local.clone(), (spec.line, spec.column));
        }

        // `../` relative imports are allowed — e.g. `api/checkout.phpx` can
        // import a helper from the package root via `import { f } from "../helpers"`.
        // The module resolver (modules.rs resolve_import_target) handles
        // is_relative paths and emits "Missing phpx module" if the target does
        // not exist. The bundler's guard_path_traversal is the security gate
        // that prevents path-escaping at bundle time.
    }

    let searchable = strip_comments_and_strings(&strip_import_lines(source, &import_lines));
    for spec in &import_specs {
        if !is_ident_used(&searchable, &spec.local) {
            warnings.push(import_warning(
                spec.line,
                find_column(&spec.line_text, &spec.local),
                spec.local.len().max(1),
                format!("Unused import '{}'.", spec.local),
                "Remove the unused import or use it in your code.",
            ));
        }
    }

    (errors, warnings)
}

pub(crate) fn parse_import_line(
    line: &str,
    raw_line: &str,
    line_number: usize,
    file_path: &str,
) -> Result<Vec<ImportSpec>, ValidationError> {
    let rest = line
        .trim_start()
        .strip_prefix("import")
        .ok_or_else(|| {
            import_error_with_suggestion(
                line_number,
                find_column(raw_line, "import"),
                line.trim().len(),
                format!("Invalid import syntax in {}.", file_path),
                "Use: import Name from 'module' or import { name } from 'module'.",
                Some("import { name } from 'module';"),
            )
        })?
        .trim_start();

    let mut default_local: Option<String> = None;
    let mut specifiers: Option<&str> = None;
    let mut after: &str;

    if rest.starts_with('{') {
        let close_idx = rest.find('}').ok_or_else(|| {
            import_error_with_suggestion(
                line_number,
                find_column(raw_line, "import"),
                line.trim().len(),
                format!("Invalid import syntax in {}.", file_path),
                "Missing closing '}' in import specifiers.",
                Some("import { name } from 'module';"),
            )
        })?;
        specifiers = Some(&rest[1..close_idx]);
        after = rest[close_idx + 1..].trim_start();
    } else {
        let (name, next) = parse_identifier(rest).ok_or_else(|| {
            import_error_with_suggestion(
                line_number,
                find_column(raw_line, "import"),
                line.trim().len(),
                format!("Invalid import syntax in {}.", file_path),
                "Expected a default import name after `import`.",
                Some("import Name from 'module';"),
            )
        })?;
        if !is_ident(name) {
            return Err(import_error(
                line_number,
                find_column(raw_line, name),
                name.len().max(1),
                format!("Invalid import name '{}' in {}.", name, file_path),
                "Import names must be valid identifiers.",
            ));
        }
        default_local = Some(name.to_string());
        let mut next = next.trim_start();
        if let Some(rest_after_comma) = next.strip_prefix(',') {
            next = rest_after_comma.trim_start();
            if !next.starts_with('{') {
                return Err(import_error_with_suggestion(
                    line_number,
                    find_column(raw_line, ","),
                    1,
                    format!("Invalid import syntax in {}.", file_path),
                    "Expected '{' after default import comma.",
                    Some("import Name, { foo } from 'module';"),
                ));
            }
            let close_idx = next.find('}').ok_or_else(|| {
                import_error_with_suggestion(
                    line_number,
                    find_column(raw_line, "import"),
                    line.trim().len(),
                    format!("Invalid import syntax in {}.", file_path),
                    "Missing closing '}' in import specifiers.",
                    Some("import Name, { foo } from 'module';"),
                )
            })?;
            specifiers = Some(&next[1..close_idx]);
            after = next[close_idx + 1..].trim_start();
        } else {
            after = next;
        }
    }

    after = after
        .strip_prefix("from")
        .ok_or_else(|| {
            import_error_with_suggestion(
                line_number,
                find_column(raw_line, "import"),
                line.trim().len(),
                format!("Invalid import syntax in {}.", file_path),
                "Expected 'from' after import specifiers.",
                Some("import { name } from 'module';"),
            )
        })?
        .trim_start();

    let (from, mut after_from) = parse_quoted_string(after).ok_or_else(|| {
        import_error_with_suggestion(
            line_number,
            find_column(raw_line, "from"),
            line.trim().len(),
            format!("Invalid import syntax in {}.", file_path),
            "Module specifier must be quoted: from 'module'.",
            Some("import { name } from 'module';"),
        )
    })?;

    after_from = after_from.trim_start();
    let mut kind = ImportKind::Phpx;
    if let Some(rest_after_as) = after_from.strip_prefix("as") {
        let kind_str = rest_after_as.trim_start();
        if !kind_str.starts_with("wasm") {
            return Err(import_error(
                line_number,
                find_column(raw_line, "as"),
                line.trim().len(),
                format!(
                    "Unknown import kind '{}' in {}.",
                    kind_str.split_whitespace().next().unwrap_or(""),
                    file_path
                ),
                "Use `as wasm` for wasm imports or omit `as`.",
            ));
        }
        kind = ImportKind::Wasm;
        after_from = kind_str.trim_start_matches("wasm").trim_start();
    }

    let after_from = after_from.trim_start_matches(';').trim();
    if !after_from.is_empty() {
        return Err(import_error_with_suggestion(
            line_number,
            find_column(raw_line, after_from),
            after_from.len().max(1),
            format!("Invalid import syntax in {}.", file_path),
            "Unexpected tokens after import statement.",
            Some("import { name } from 'module';"),
        ));
    }

    let mut specs = Vec::new();
    if let Some(local) = default_local {
        if kind == ImportKind::Wasm {
            return Err(import_error_with_suggestion(
                line_number,
                find_column(raw_line, &local),
                local.len().max(1),
                format!(
                    "Default imports are not supported for wasm modules in {}.",
                    file_path
                ),
                "Use named wasm imports: import { fn } from '@user/mod' as wasm.",
                Some("import { fn } from '@user/mod' as wasm;"),
            ));
        }
        specs.push(ImportSpec {
            imported: "default".to_string(),
            local: local.clone(),
            from: from.clone(),
            kind,
            line: line_number,
            column: find_column(raw_line, &local),
            line_text: raw_line.to_string(),
        });
    }

    if let Some(specifiers) = specifiers {
        let spec_list: Vec<&str> = specifiers
            .split(',')
            .map(|spec| spec.trim())
            .filter(|spec| !spec.is_empty())
            .collect();

        if spec_list.is_empty() {
            return Err(import_error_with_suggestion(
                line_number,
                find_column(raw_line, "import"),
                line.trim().len(),
                format!("Empty import list in {}.", file_path),
                "Add at least one import specifier.",
                Some("import { name } from 'module';"),
            ));
        }

        for spec in spec_list {
            let parts: Vec<&str> = spec.split_whitespace().collect();
            let (imported, local) = match parts.as_slice() {
                [name] => (*name, *name),
                [name, as_kw, alias] if *as_kw == "as" => (*name, *alias),
                _ => {
                    return Err(import_error(
                        line_number,
                        find_column(raw_line, spec),
                        spec.len().max(1),
                        format!("Invalid import specifier '{}' in {}.", spec, file_path),
                        "Use `name` or `name as alias` inside the import list.",
                    ));
                }
            };

            if !is_ident(imported) || !is_ident(local) {
                return Err(import_error(
                    line_number,
                    find_column(raw_line, spec),
                    spec.len().max(1),
                    format!("Invalid import specifier '{}' in {}.", spec, file_path),
                    "Import names must be valid identifiers.",
                ));
            }

            specs.push(ImportSpec {
                imported: imported.to_string(),
                local: local.to_string(),
                from: from.clone(),
                kind,
                line: line_number,
                column: find_column(raw_line, local),
                line_text: raw_line.to_string(),
            });
        }
    } else if specs.is_empty() {
        return Err(import_error_with_suggestion(
            line_number,
            find_column(raw_line, "import"),
            line.trim().len(),
            format!("Empty import list in {}.", file_path),
            "Add a default import or a named import list.",
            Some("import Name from 'module';"),
        ));
    }

    Ok(specs)
}

fn import_error(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
) -> ValidationError {
    import_error_with_suggestion(line, column, underline_length, message, help_text, None)
}

fn import_error_with_suggestion(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
    suggestion: Option<&str>,
) -> ValidationError {
    ValidationError {
        kind: ErrorKind::ImportError,
        line,
        column,
        message,
        help_text: help_text.to_string(),
        suggestion: suggestion.map(|value| value.to_string()),
        underline_length: underline_length.max(1),
        severity: Severity::Error,
    }
}

fn import_warning(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
    help_text: &str,
) -> ValidationWarning {
    ValidationWarning {
        kind: ErrorKind::ImportError,
        line,
        column,
        message,
        help_text: help_text.to_string(),
        suggestion: None,
        underline_length: underline_length.max(1),
        severity: Severity::Warning,
    }
}

pub(crate) fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn parse_identifier(input: &str) -> Option<(&str, &str)> {
    let mut chars = input.char_indices();
    let (start_idx, first) = chars.next()?;
    if start_idx != 0 {
        return None;
    }
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    let mut end = first.len_utf8();
    for (idx, ch) in chars {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    Some((&input[..end], &input[end..]))
}

pub(crate) fn parse_quoted_string(input: &str) -> Option<(String, &str)> {
    let mut chars = input.char_indices();
    let (_, quote) = chars.next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    let mut end_idx = None;
    for (idx, ch) in chars {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            end_idx = Some(idx);
            break;
        }
        out.push(ch);
    }
    let end_idx = end_idx?;
    let rest = &input[end_idx + quote.len_utf8()..];
    Some((out, rest))
}

pub(crate) fn strip_php_tags_inline(line: &str) -> String {
    line.replace("<?phpx", "")
        .replace("<?php", "")
        .replace("<?", "")
        .replace("?>", "")
}

pub(crate) fn frontmatter_bounds(lines: &[&str]) -> Option<(usize, usize)> {
    if lines.is_empty() {
        return None;
    }
    let mut i = 0;
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    if i >= lines.len() {
        return None;
    }
    let mut first = lines[i];
    if let Some(stripped) = first.strip_prefix('\u{feff}') {
        first = stripped;
    }
    if first.trim() != "---" {
        return None;
    }
    let start = i;
    i += 1;
    for idx in i..lines.len() {
        if lines[idx].trim() == "---" {
            return Some((start, idx));
        }
    }
    None
}

pub(crate) fn consume_comment_line(trimmed: &str, in_block: &mut bool) -> bool {
    if *in_block {
        if let Some(end_idx) = trimmed.find("*/") {
            let rest = trimmed[end_idx + 2..].trim();
            *in_block = false;
            return rest.is_empty();
        }
        return true;
    }

    if trimmed.starts_with("/*") {
        if trimmed.contains("*/") {
            return true;
        }
        *in_block = true;
        return true;
    }

    trimmed.starts_with("//") || trimmed.starts_with('#')
}

fn strip_import_lines(source: &str, import_lines: &HashSet<usize>) -> String {
    let mut out = String::new();
    for (idx, line) in source.lines().enumerate() {
        if import_lines.contains(&idx) {
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub(crate) fn strip_comments_and_strings(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    while let Some(ch) = chars.next() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
                out.push('\n');
            }
            continue;
        }
        if in_block_comment {
            if ch == '*' {
                if let Some('/') = chars.peek().copied() {
                    chars.next();
                    in_block_comment = false;
                }
            }
            continue;
        }
        if in_single {
            if ch == '\\' {
                chars.next();
                continue;
            }
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            if ch == '\\' {
                chars.next();
                continue;
            }
            if ch == '"' {
                in_double = false;
            }
            continue;
        }

        if ch == '/' {
            if let Some('/') = chars.peek().copied() {
                chars.next();
                in_line_comment = true;
                continue;
            }
            if let Some('*') = chars.peek().copied() {
                chars.next();
                in_block_comment = true;
                continue;
            }
        }
        if ch == '#' {
            in_line_comment = true;
            continue;
        }
        if ch == '\'' {
            in_single = true;
            continue;
        }
        if ch == '"' {
            in_double = true;
            continue;
        }

        out.push(ch);
    }
    out
}

fn is_ident_used(source: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = source.as_bytes();
    let needle = name.as_bytes();
    let mut idx = 0;
    while idx + needle.len() <= bytes.len() {
        if let Some(pos) = source[idx..].find(name) {
            let start = idx + pos;
            let end = start + needle.len();
            let before = if start == 0 {
                None
            } else {
                Some(bytes[start - 1])
            };
            let after = if end >= bytes.len() {
                None
            } else {
                Some(bytes[end])
            };
            let valid_before = before.map_or(true, |b| !is_ident_char(b));
            let valid_after = after.map_or(true, |b| !is_ident_char(b));
            if valid_before && valid_after {
                return true;
            }
            idx = end;
        } else {
            break;
        }
    }
    false
}

fn is_ident_char(byte: u8) -> bool {
    (byte as char).is_ascii_alphanumeric() || byte == b'_'
}

pub(crate) fn find_column(line: &str, needle: &str) -> usize {
    line.find(needle).map(|idx| idx + 1).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::validate_imports;

    #[test]
    fn parent_relative_import_is_allowed() {
        // Regression for issue #36: `../helpers` from a subdir file must not
        // be rejected by the validator.  The bundler's guard_path_traversal is
        // the security gate; the validator should only reject syntactically
        // invalid import paths.
        let src = r#"import { client_ip } from "../helpers";
$x = client_ip();"#;
        let (errors, _warnings) = validate_imports(src, "api/checkout.phpx");
        let parent_errors: Vec<_> = errors.iter().filter(|e| e.message.contains("..")).collect();
        assert!(
            parent_errors.is_empty(),
            "expected no errors for ../helpers import, got: {:?}",
            parent_errors
        );
    }

    #[test]
    fn same_dir_relative_import_is_allowed() {
        let src = r#"import { helper } from "./utils";
$x = helper();"#;
        let (errors, _warnings) = validate_imports(src, "api/checkout.phpx");
        assert!(
            errors.is_empty(),
            "expected no errors for ./utils import, got: {:?}",
            errors
        );
    }

    #[test]
    fn duplicate_local_import_is_rejected() {
        let src = r#"import { foo } from "array";
import { foo } from "string";
$x = foo();"#;
        let (errors, _warnings) = validate_imports(src, "test.phpx");
        assert!(
            errors.iter().any(|e| e.message.contains("Duplicate")),
            "expected duplicate import error, got: {:?}",
            errors
        );
    }
}
