//! Shared compile-and-report helper for CLI commands.
//!
//! All DekaScript-facing CLI commands should go through this module so that
//! validation diagnostics are printed with the rich Rust/Gleam/Elm-style
//! formatter and never buried under generic wrappers like
//! `Run failed: Failed to load module:`.

use core::Context;
use deka_compile::{CompilerVersion, compile_to_js, format_diagnostic};
use deka_js::{
    CompileError, CompileOutcome,
    compile_phpx_source_to_js_with_warnings_detailed,
};

/// Module metadata for either compiler version.
///
/// v1 and v2 use different metadata shapes. This enum lets callers pass the
/// right metadata for the compiler they selected without leaking v1 types into
/// the v2 path.
pub enum ModuleMeta {
    V1(deka_js::SourceModuleMeta),
    V2(deka_compile::SourceModuleMeta),
}

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
    meta: ModuleMeta,
    compiler: CompilerVersion,
) -> Result<CompileReport, String> {
    match compiler {
        CompilerVersion::V1 => {
            let meta = match meta {
                ModuleMeta::V1(meta) => meta,
                ModuleMeta::V2(_) => {
                    return Err("v2 module metadata passed to v1 compiler".to_string())
                }
            };
            match compile_phpx_source_to_js_with_warnings_detailed(source, input, meta) {
                Ok(CompileOutcome { js, warnings }) => Ok(CompileReport { js, warnings }),
                Err(CompileError::Validation { diagnostics }) => Err(diagnostics),
                Err(CompileError::Other(msg)) => Err(msg),
            }
        }
        CompilerVersion::V2 => {
            let _meta = match meta {
                ModuleMeta::V2(meta) => meta,
                ModuleMeta::V1(_) => {
                    return Err("v1 module metadata passed to v2 compiler".to_string())
                }
            };
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
    }
}

/// Compile and return only the emitted JS, discarding warnings.
pub fn compile_js_or_report(
    source: &str,
    input: &str,
    meta: ModuleMeta,
    compiler: CompilerVersion,
) -> Result<String, String> {
    compile_or_report(source, input, meta, compiler).map(|report| report.js)
}
