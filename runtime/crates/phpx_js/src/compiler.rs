use crate::{SourceModuleMeta, emitter::JsSubsetEmitter};
use bumpalo::Bump;
use modules_php::compiler_api::{compile_deka, compile_phpx, compile_phpx_internal};
use modules_php::validation::{format_multiple_errors, format_validation_warning};
use php_rs::parser::ast::Program;
use std::path::Path;

/// Marker that lets runtime callers distinguish a pre-rendered validation
/// report from a low-level runtime error. The marker is stripped before the
/// report is printed to stdout/stderr.
pub const DEKA_VALIDATION_ERROR_MARKER: &str = "DEKA_VALIDATION_ERROR:";

/// A compile failure that carries enough context for callers to decide how to
/// present it. Validation errors are already rendered with file/line/column
/// diagnostics; other errors are plain strings from IO, project-layout, or
/// emitter failures.
#[derive(Debug, Clone)]
pub enum CompileError {
    Validation { diagnostics: String },
    Other(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Validation { diagnostics } => write!(f, "{}", diagnostics),
            CompileError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for CompileError {}

impl CompileError {
    /// True if this error is a rendered validation diagnostic.
    pub fn is_validation(&self) -> bool {
        matches!(self, CompileError::Validation { .. })
    }

    /// True if the error text begins with the validation marker.
    pub fn from_marked_string(s: String) -> Self {
        if let Some(rest) = s.strip_prefix(DEKA_VALIDATION_ERROR_MARKER) {
            CompileError::Validation {
                diagnostics: rest.to_string(),
            }
        } else {
            CompileError::Other(s)
        }
    }
}

/// Result of a successful compile that also carries warning-severity
/// diagnostics (deka#59). `deka build`'s exit-code path does not use this --
/// see `compile_phpx_source_to_js` below, which is untouched and still
/// discards warnings exactly as it did before this change (that exit-code
/// behavior belongs to a different lane fixing deka#5).
pub struct CompileOutcome {
    pub js: String,
    /// Pre-rendered, human-readable warning text (same renderer used for
    /// errors, just yellow instead of red) -- one entry per warning.
    pub warnings: Vec<String>,
}

/// Compile `source` to JS. Warnings are silently discarded on success --
/// this is the function `deka build` calls, and its Ok/Err (and therefore
/// exit-code) behavior must stay exactly as it was pre-deka#59. Callers that
/// want to see warnings (e.g. `deka check`) should call
/// `compile_phpx_source_to_js_with_warnings` instead.
pub fn compile_phpx_source_to_js(
    source: &str,
    input: &str,
    meta: SourceModuleMeta,
) -> Result<String, String> {
    compile_phpx_source_to_js_with_warnings(source, input, meta).map(|outcome| outcome.js)
}

/// Like `compile_phpx_source_to_js`, but on success also returns rendered
/// warning-severity diagnostics instead of discarding them. `Err` behavior
/// (including the exact formatted string) is unchanged.
pub fn compile_phpx_source_to_js_with_warnings(
    source: &str,
    input: &str,
    meta: SourceModuleMeta,
) -> Result<CompileOutcome, String> {
    compile_phpx_source_to_js_with_warnings_detailed(source, input, meta)
        .map_err(|err| err.to_string())
}

/// Detailed compile API that distinguishes validation diagnostics from other
/// failures. Prefer this in new callers that need to present rich errors.
pub fn compile_phpx_source_to_js_with_warnings_detailed(
    source: &str,
    input: &str,
    mut meta: SourceModuleMeta,
) -> Result<CompileOutcome, CompileError> {
    let arena = Bump::new();
    let path = Path::new(input);
    let is_ds = path.extension().and_then(|ext| ext.to_str()) == Some("ds");
    meta.is_ds = meta.is_ds || is_ds;
    let result = if is_ds {
        compile_deka(source, input, &arena)
    } else if is_internal_phpx_path(path) {
        compile_phpx_internal(source, input, &arena)
    } else {
        compile_phpx(source, input, &arena)
    };
    if !result.errors.is_empty() {
        let formatted = format_multiple_errors(source, input, &result.errors, &result.warnings);
        return Err(CompileError::Validation { diagnostics: formatted });
    }

    let warnings: Vec<String> = result
        .warnings
        .iter()
        .map(|warning| format_validation_warning(source, input, warning))
        .collect();

    let is_ds = meta.is_ds;
    let js = if let Some(program) = result.ast {
        match emit_js_from_ast(&program, source.as_bytes(), meta) {
            Ok(emitted) => emitted,
            Err(reason) => {
                if is_ds {
                    return Err(CompileError::Other(reason));
                }
                emit_js_scaffold_with_reason(source, input, &reason)
            }
        }
    } else {
        if is_ds {
            return Err(CompileError::Other(
                "no AST available after validation".to_string(),
            ));
        }
        emit_js_scaffold_with_reason(source, input, "no AST available after validation")
    };

    Ok(CompileOutcome { js, warnings })
}

fn is_internal_phpx_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.starts_with("php_modules/") || normalized.contains("/php_modules/")
}

pub fn emit_js_from_ast(
    program: &Program<'_>,
    source: &[u8],
    meta: SourceModuleMeta,
) -> Result<String, String> {
    let (js, warnings) = emit_js_from_ast_with_warnings(program, source, meta)?;
    for w in &warnings {
        eprintln!("[phpx warning] {}", w);
    }
    Ok(js)
}

/// Like `emit_js_from_ast` but returns collected warnings instead of printing
/// them.  Used by tests to assert on warning content.
pub fn emit_js_from_ast_with_warnings(
    program: &Program<'_>,
    source: &[u8],
    meta: SourceModuleMeta,
) -> Result<(String, Vec<String>), String> {
    let mut emitter = JsSubsetEmitter::new(source, meta);
    emitter.emit_program(program)?;
    let warnings = emitter.warnings.clone();
    Ok((emitter.finish(), warnings))
}

pub fn emit_js_scaffold_with_reason(source: &str, file_path: &str, reason: &str) -> String {
    let escaped = serde_json::to_string(source).unwrap_or_else(|_| "\"\"".to_string());
    let escaped_path =
        serde_json::to_string(file_path).unwrap_or_else(|_| "\"unknown.phpx\"".to_string());
    let escaped_reason =
        serde_json::to_string(reason).unwrap_or_else(|_| "\"unknown\"".to_string());

    format!(
        "// Generated by deka build. Do not edit manually.\n\
// Source: {file_path}\n\
// Target semantics: JavaScript runtime semantics.\n\
// Fallback scaffold used because subset emitter could not lower this file.\n\
export const phpxBuildMode = \"scaffold\";\n\
export const phpxTargetSemantics = \"js\";\n\
export const phpxBuildReason = {escaped_reason};\n\
export const phpxSource = {escaped};\n\
export const phpxFile = {escaped_path};\n\
\n\
export async function runPhpx(runtime, props = {{}}) {{\n\
  if (!runtime || typeof runtime.executePhpx !== 'function') {{\n\
    throw new Error('runtime.executePhpx(source, file, props) is required');\n\
  }}\n\
  return await runtime.executePhpx(phpxSource, phpxFile, props);\n\
}}\n",
    )
}
