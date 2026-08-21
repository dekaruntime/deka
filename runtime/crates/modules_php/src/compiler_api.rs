//! PHPX compiler/validation entry points for tooling (LSP, tests, etc.).

use bumpalo::Bump;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};

use crate::validation::cypher::validate_cypher;
use crate::validation::exports::validate_exports;
use crate::validation::generics::validate_generics;
use crate::validation::hoisting::validate_hoisting;
use crate::validation::imports::validate_imports;
use crate::validation::imports::{
    ImportKind, ImportSpec, consume_comment_line, frontmatter_bounds, strip_php_tags_inline,
};
use crate::validation::jsx::{
    validate_components, validate_frontmatter, validate_jsx_expressions, validate_jsx_syntax,
    validate_template_section,
};
use crate::validation::modules::{
    resolve_modules_root, validate_module_resolution, validate_target_capabilities,
    validate_wasm_imports,
};
use crate::validation::patterns::validate_match_exhaustiveness;
use crate::validation::phpx_rules::{
    validate_no_exceptions, validate_no_namespace, validate_no_null, validate_no_oop,
};
use crate::validation::structs::{validate_struct_definitions, validate_struct_literals};
use crate::validation::syntax::validate_syntax;
use crate::validation::type_checker::{check_types, check_types_with_externals};
use crate::validation::type_syntax::validate_type_annotations;
use crate::validation::{ErrorKind, Severity, ValidationError, ValidationResult};
use php_rs::phpx::typeck::{ExternalFunctionSig, external_functions_from_stub};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Compile and validate a PHPX source file.
///
/// Returns a `ValidationResult` with errors, warnings, and the parsed AST
/// (if no syntax errors were encountered). Callers should provide a bump
/// arena for AST allocations.
pub fn compile_phpx<'a>(source: &str, file_path: &str, arena: &'a Bump) -> ValidationResult<'a> {
    compile_phpx_with_mode(source, file_path, arena, ParserMode::Phpx, true, false)
}

pub fn compile_deka<'a>(source: &str, file_path: &str, arena: &'a Bump) -> ValidationResult<'a> {
    compile_phpx_with_mode(source, file_path, arena, ParserMode::Ds, true, false)
}

/// Compile a single DekaScript module that is part of a virtual project.
///
/// This skips filesystem-based module resolution, target capability checks,
/// and wasm import validation because the project compiler resolves imports
/// against the virtual file system itself.
pub fn compile_deka_project_module<'a>(
    source: &str,
    file_path: &str,
    arena: &'a Bump,
) -> ValidationResult<'a> {
    compile_phpx_with_mode(source, file_path, arena, ParserMode::Ds, true, true)
}

pub fn compile_phpx_internal<'a>(
    source: &str,
    file_path: &str,
    arena: &'a Bump,
) -> ValidationResult<'a> {
    compile_phpx_with_mode(source, file_path, arena, ParserMode::PhpxInternal, false, false)
}

fn compile_phpx_with_mode<'a>(
    source: &str,
    file_path: &str,
    arena: &'a Bump,
    mode: ParserMode,
    strict: bool,
    skip_module_resolution: bool,
) -> ValidationResult<'a> {
    let parser_source = preprocess_source(source, mode);
    let lexer = Lexer::new(parser_source.as_bytes());
    let mut parser = Parser::new_with_mode(lexer, arena, mode);
    let program = parser.parse_program();

    let mut errors = validate_syntax(source, &program, file_path);
    let mut warnings = Vec::new();
    let has_parse_errors = !errors.is_empty();

    if mode == ParserMode::Ds && !has_parse_errors {
        let (hoist_errors, hoist_warnings) = validate_hoisting(&program, source, file_path);
        errors.extend(hoist_errors);
        warnings.extend(hoist_warnings);
    }

    let (import_errors, import_warnings) = validate_imports(source, file_path);
    errors.extend(import_errors);
    warnings.extend(import_warnings);

    let export_errors = validate_exports(source, file_path, &program, mode == ParserMode::Ds);
    errors.extend(export_errors);

    let (mut wasm_functions, wasm_errors) = collect_wasm_stub_signatures(source, file_path, arena);
    errors.extend(wasm_errors);

    if strict {
        errors.extend(validate_type_annotations(&program, source));

        let (type_errors, type_warnings) = if wasm_functions.is_empty() {
            check_types(&program, source, Some(file_path))
        } else {
            check_types_with_externals(&program, source, Some(file_path), &wasm_functions)
        };
        errors.extend(type_errors);
        warnings.extend(type_warnings);

        let (generic_errors, generic_warnings) = validate_generics(&program, source);
        errors.extend(generic_errors);
        warnings.extend(generic_warnings);

        let is_ds = mode == ParserMode::Ds;
        errors.extend(validate_no_null(&program, source, is_ds));
        errors.extend(validate_no_exceptions(&program, source, is_ds));
        errors.extend(validate_no_oop(&program, source, is_ds));
        errors.extend(validate_no_namespace(&program, source, is_ds));

        errors.extend(validate_struct_definitions(&program, source));
        errors.extend(validate_struct_literals(&program, source));
    }

    errors.extend(validate_frontmatter(source, file_path));
    errors.extend(validate_template_section(source, file_path));
    errors.extend(validate_jsx_syntax(&program, source));
    errors.extend(validate_jsx_expressions(&program, source));
    errors.extend(validate_components(&program, source));

    if !skip_module_resolution {
        errors.extend(validate_module_resolution(source, file_path));
        errors.extend(validate_target_capabilities(source, file_path));
        errors.extend(validate_wasm_imports(source, file_path));
    }

    errors.extend(validate_match_exhaustiveness(&program, source));
    errors.extend(validate_cypher(&program, source));

    if has_parse_errors {
        wasm_functions.clear();
    }

    ValidationResult {
        errors,
        warnings,
        ast: if has_parse_errors {
            None
        } else {
            Some(program)
        },
        wasm_functions,
    }
}

fn preprocess_source(source: &str, mode: ParserMode) -> String {
    let line_refs: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_bounds(&line_refs);
    let mut output = String::with_capacity(source.len());
    let mut line_index = 0usize;

    for segment in source.split_inclusive('\n') {
        let (in_frontmatter, is_delim) = if let Some((start, end)) = bounds {
            if start == end {
                // Script/template separator: keep code before the delimiter,
                // mask the delimiter line and the template section after it.
                (line_index < start, line_index == start)
            } else {
                (line_index > start && line_index < end,
                 line_index == start || line_index == end)
            }
        } else {
            (true, false)
        };

        let clean = strip_php_tags_inline(segment);
        let trimmed = clean.trim();

        let mut masked = false;
        if !in_frontmatter || is_delim {
            masked = true;
        } else if trimmed.starts_with("import ") {
            masked = true;
        } else if trimmed.starts_with("export {") {
            masked = true;
        } else if mode != ParserMode::Ds && trimmed.starts_with("export ")
            && !trimmed.starts_with("export function")
            && !trimmed.starts_with("export async function")
        {
            masked = true;
        } else if mode == ParserMode::Ds && trimmed.starts_with("export ") {
            output.push_str(&mask_export_keyword(segment));
            line_index += 1;
            continue;
        }

        if masked {
            output.push_str(&mask_segment(segment));
        } else if trimmed.starts_with("export function")
            || trimmed.starts_with("export async function")
        {
            output.push_str(&mask_export_keyword(segment));
        } else {
            output.push_str(segment);
        }

        line_index += 1;
    }

    output
}

fn mask_segment(segment: &str) -> String {
    segment
        .chars()
        .map(|ch| if ch == '\n' { '\n' } else { ' ' })
        .collect()
}

fn mask_export_keyword(segment: &str) -> String {
    if let Some(idx) = segment.find("export") {
        let mut out = String::with_capacity(segment.len());
        out.push_str(&segment[..idx]);
        out.push_str("      ");
        out.push_str(&segment[idx + 6..]);
        return out;
    }
    segment.to_string()
}

fn collect_wasm_stub_signatures(
    source: &str,
    file_path: &str,
    arena: &Bump,
) -> (HashMap<String, ExternalFunctionSig>, Vec<ValidationError>) {
    let mut errors = Vec::new();
    let specs = collect_wasm_import_specs(source, file_path);
    if specs.is_empty() {
        return (HashMap::new(), errors);
    }

    let mut out: HashMap<String, ExternalFunctionSig> = HashMap::new();
    let mut stub_cache: HashMap<PathBuf, HashMap<String, ExternalFunctionSig>> = HashMap::new();

    for spec in specs {
        let Some(stub_path) = resolve_wasm_stub_path(file_path, &spec) else {
            continue;
        };
        let entry = if let Some(cached) = stub_cache.get(&stub_path) {
            Some(cached)
        } else {
            let stub_source = match std::fs::read_to_string(&stub_path) {
                Ok(src) => src,
                Err(err) => {
                    errors.push(wasm_stub_error(
                        spec.line,
                        spec.column,
                        spec.from.len().max(1),
                        format!("Failed to read wasm stub {}: {}", stub_path.display(), err),
                    ));
                    continue;
                }
            };
            let processed = preprocess_stub_source(&stub_source);
            let program = parse_stub_program(&processed, arena);
            if !program.errors.is_empty() {
                let mut parse_errors = crate::validation::parse_errors_to_validation_errors(
                    &stub_source,
                    program.errors,
                );
                for err in &mut parse_errors {
                    err.kind = ErrorKind::WasmError;
                    err.message = format!("Wasm stub parse error: {}", err.message);
                    err.help_text = "Regenerate stubs with `deka wasm stubs`.".to_string();
                }
                errors.extend(parse_errors);
                continue;
            }
            let sigs = external_functions_from_stub(&program, processed.as_bytes());
            stub_cache.insert(stub_path.clone(), sigs);
            stub_cache.get(&stub_path)
        };

        let Some(exports) = entry else {
            continue;
        };
        if let Some(sig) = exports.get(&spec.imported) {
            out.insert(spec.local.clone(), sig.clone());
        }
    }

    (out, errors)
}

fn wasm_stub_error(
    line: usize,
    column: usize,
    underline_length: usize,
    message: String,
) -> ValidationError {
    ValidationError {
        kind: ErrorKind::WasmError,
        line,
        column,
        message,
        help_text: "Regenerate stubs with `deka wasm stubs`.".to_string(),
        suggestion: None,
        underline_length,
        severity: Severity::Error,
    }
}

fn collect_wasm_import_specs(source: &str, _file_path: &str) -> Vec<ImportSpec> {
    let lines: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_bounds(&lines);
    let scan_end = bounds.map(|(_, end)| end).unwrap_or(lines.len());
    let mut in_block_comment = false;
    let mut specs = Vec::new();

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
        if !trimmed.starts_with("import ") || !trimmed.contains(" as wasm") {
            continue;
        }

        let (open, close) = match (trimmed.find('{'), trimmed.find('}')) {
            (Some(open), Some(close)) if close > open => (open, close),
            _ => continue,
        };
        let spec_part = &trimmed[open + 1..close];
        let from_idx = match trimmed.find("from") {
            Some(idx) => idx,
            None => continue,
        };
        let after_from = &trimmed[from_idx + 4..];
        let (from, _) = match parse_module_spec(after_from) {
            Some(value) => value,
            None => continue,
        };

        for spec in spec_part.split(',') {
            let spec_trim = spec.trim();
            if spec_trim.is_empty() {
                continue;
            }
            let (imported, local) = if let Some((left, right)) = spec_trim.split_once(" as ") {
                (left.trim(), right.trim())
            } else {
                (spec_trim, spec_trim)
            };
            if imported.is_empty() || local.is_empty() {
                continue;
            }
            let column = line.find(local).map(|idx| idx + 1).unwrap_or(1);
            specs.push(ImportSpec {
                imported: imported.to_string(),
                local: local.to_string(),
                from: from.to_string(),
                kind: ImportKind::Wasm,
                line: idx + 1,
                column,
                line_text: line.to_string(),
            });
        }
    }

    specs
}

fn parse_module_spec(input: &str) -> Option<(String, &str)> {
    let trimmed = input.trim_start();
    let quote_idx = trimmed.find(&['\'', '"'][..])?;
    let quote_char = trimmed.chars().nth(quote_idx)?;
    let after_quote = &trimmed[quote_idx + 1..];
    let end_idx = after_quote.find(quote_char)?;
    let module = &after_quote[..end_idx];
    Some((module.to_string(), &after_quote[end_idx + 1..]))
}

fn resolve_wasm_stub_path(file_path: &str, spec: &ImportSpec) -> Option<PathBuf> {
    let modules_root = resolve_modules_root(file_path)?;
    let raw = spec.from.trim();
    let is_relative = raw.starts_with('.');
    let is_project_alias = raw.starts_with("@/");
    let spec_path = raw.strip_prefix("@/").unwrap_or(raw);
    let base_dir = if is_relative {
        Path::new(file_path)
            .parent()
            .unwrap_or(modules_root.as_path())
            .to_path_buf()
    } else if is_project_alias {
        modules_root
            .parent()
            .unwrap_or(modules_root.as_path())
            .to_path_buf()
    } else {
        modules_root.clone()
    };
    let root_path = base_dir.join(spec_path);
    let allowed_root = if is_project_alias {
        modules_root.parent().unwrap_or(modules_root.as_path())
    } else {
        modules_root.as_path()
    };
    if root_path.strip_prefix(allowed_root).ok().is_none() {
        return None;
    }
    let manifest_path = root_path.join("deka.json");
    let raw = std::fs::read_to_string(&manifest_path).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    let stub_rel = parsed
        .get("stubs")
        .and_then(|v| v.as_str())
        .unwrap_or("module.d.phpx");
    Some(root_path.join(stub_rel))
}

fn parse_stub_program<'a>(source: &str, arena: &'a Bump) -> php_rs::parser::ast::Program<'a> {
    let lexer = Lexer::new(source.as_bytes());
    let mut parser = Parser::new_with_mode(lexer, arena, ParserMode::Phpx);
    parser.parse_program()
}

fn preprocess_stub_source(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    for segment in source.split_inclusive('\n') {
        let clean = strip_php_tags_inline(segment);
        let trimmed = clean.trim_start();
        let mut line = if trimmed.starts_with("export ") {
            if let Some(idx) = clean.find("export") {
                let mut out = String::with_capacity(clean.len());
                out.push_str(&clean[..idx]);
                out.push_str("      ");
                out.push_str(&clean[idx + 6..]);
                out
            } else {
                clean
            }
        } else {
            clean
        };

        let has_newline = line.ends_with('\n');
        let line_trimmed_end = line.trim_end_matches('\n');
        let trimmed_no_ws = line_trimmed_end.trim_start();
        if trimmed_no_ws.starts_with("function ") && line_trimmed_end.trim_end().ends_with(';') {
            let mut replaced = line_trimmed_end
                .trim_end()
                .trim_end_matches(';')
                .to_string();
            replaced.push_str(" {}");
            line = replaced;
            if has_newline {
                line.push('\n');
            }
        }

        output.push_str(&line);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::compile_deka;
    use bumpalo::Bump;

    #[test]
    fn ds_export_const_validates_through_compiler_api() {
        let arena = Bump::new();
        let result = compile_deka("export const answer = 42;", "lesson.ds", &arena);
        assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);
        assert!(result.ast.is_some(), "expected validated AST");
    }
}
