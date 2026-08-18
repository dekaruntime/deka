//! Shared compile-and-report helper for CLI commands.
//!
//! All DekaScript-facing CLI commands should go through this module so that
//! validation diagnostics are printed with the rich Rust/Gleam/Elm-style
//! formatter and never buried under generic wrappers like
//! `Run failed: Failed to load module:`.

use phpx_js::{
    CompileError, CompileOutcome, SourceModuleMeta,
    compile_phpx_source_to_js_with_warnings_detailed,
};

/// Outcome of a successful compile that the CLI may want to act on.
pub struct CompileReport {
    pub js: String,
    pub warnings: Vec<String>,
}

/// Compile a DekaScript/PHPX source and return the emitted JS plus warnings.
///
/// Validation errors are returned as the already-formatted diagnostic string,
/// suitable for printing directly via `stdio::error` without further wrapping.
/// All other errors are passed through unchanged.
pub fn compile_or_report(
    source: &str,
    input: &str,
    meta: SourceModuleMeta,
) -> Result<CompileReport, String> {
    match compile_phpx_source_to_js_with_warnings_detailed(source, input, meta) {
        Ok(CompileOutcome { js, warnings }) => Ok(CompileReport { js, warnings }),
        Err(CompileError::Validation { diagnostics }) => Err(diagnostics),
        Err(CompileError::Other(msg)) => Err(msg),
    }
}

/// Compile and return only the emitted JS, discarding warnings.
pub fn compile_js_or_report(source: &str, input: &str, meta: SourceModuleMeta) -> Result<String, String> {
    compile_or_report(source, input, meta).map(|report| report.js)
}
