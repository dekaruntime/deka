//! Shared compile-and-report helper for CLI commands.
//!
//! All DekaScript-facing CLI commands should go through this module so that
//! validation diagnostics are printed with the rich Rust/Gleam/Elm-style
//! formatter and never buried under generic wrappers like
//! `Run failed: Failed to load module:`.

use core::Context;
use deka_compile::{CompilerVersion, compile_to_js, format_diagnostic};
use deka_js::{
    CompileError, CompileOutcome, SourceModuleMeta,
    compile_phpx_source_to_js_with_warnings_detailed,
};

/// Outcome of a successful compile that the CLI may want to act on.
pub struct CompileReport {
    pub js: String,
    pub warnings: Vec<String>,
}

/// Resolve the compiler version requested by the user.
///
/// The `--compiler <v1|v2>` command-line parameter takes precedence; if it is
/// absent, the `DEKA_COMPILER` environment variable is consulted.  Anything
/// other than `v2` defaults to the legacy v1 compiler.
pub fn compiler_version_from_context(context: &Context) -> CompilerVersion {
    if let Some(value) = context.args.params.get("--compiler") {
        if value.trim().eq_ignore_ascii_case("v2") {
            return CompilerVersion::V2;
        }
        return CompilerVersion::V1;
    }

    if let Ok(value) = std::env::var("DEKA_COMPILER") {
        if value.trim().eq_ignore_ascii_case("v2") {
            return CompilerVersion::V2;
        }
    }

    CompilerVersion::V1
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
    compiler: CompilerVersion,
) -> Result<CompileReport, String> {
    match compiler {
        CompilerVersion::V1 => {
            match compile_phpx_source_to_js_with_warnings_detailed(source, input, meta) {
                Ok(CompileOutcome { js, warnings }) => Ok(CompileReport { js, warnings }),
                Err(CompileError::Validation { diagnostics }) => Err(diagnostics),
                Err(CompileError::Other(msg)) => Err(msg),
            }
        }
        CompilerVersion::V2 => match compile_to_js(source, input) {
            Ok(result) => {
                let warnings = result
                    .diagnostics
                    .iter()
                    .map(format_diagnostic)
                    .collect();
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
        },
    }
}

/// Compile and return only the emitted JS, discarding warnings.
pub fn compile_js_or_report(
    source: &str,
    input: &str,
    meta: SourceModuleMeta,
    compiler: CompilerVersion,
) -> Result<String, String> {
    compile_or_report(source, input, meta, compiler).map(|report| report.js)
}
