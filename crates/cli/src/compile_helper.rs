//! Shared compile-and-report helper for CLI commands.
//!
//! All DekaScript-facing CLI commands should go through this module so that
//! validation diagnostics are printed with the rich Rust/Gleam/Elm-style
//! formatter and never buried under generic wrappers like
//! `Run failed: Failed to load module:`.

use deka_compile::{compile_to_js, format_diagnostic};

/// Module metadata extracted from a DekaScript source file.
pub use deka_compile::SourceModuleMeta as ModuleMeta;

/// Outcome of a successful compile that the CLI may want to act on.
pub struct CompileReport {
    pub js: String,
    pub warnings: Vec<String>,
}

/// Compile a DekaScript source and return the emitted JS plus warnings.
///
/// Validation errors are returned as the already-formatted diagnostic string,
/// suitable for printing directly via `stdio::error` without further wrapping.
/// All other errors are passed through unchanged.
pub fn compile_or_report(source: &str, input: &str) -> Result<CompileReport, String> {
    match compile_to_js(source, input) {
        Ok(result) => {
            let warnings = result.diagnostics.iter().map(format_diagnostic).collect();
            Ok(CompileReport {
                js: result.js,
                warnings,
            })
        }
        Err(diagnostics) => Err(diagnostics
            .iter()
            .map(format_diagnostic)
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

/// Compile and return only the emitted JS, discarding warnings.
pub fn compile_js_or_report(source: &str, input: &str) -> Result<String, String> {
    compile_or_report(source, input).map(|report| report.js)
}
