//! Browser compiler ABI for Deka source languages.
//!
//! The neutral `deka_compiler_*` exports are the only browser-facing ABI.

use std::alloc::{Layout, alloc, dealloc};
use std::{ptr, slice, str};

use bumpalo::Bump;
use modules_php::{
    compiler_api::compile_deka,
    validation::{Severity, ValidationError, ValidationWarning},
};
use serde::Serialize;

/// Alignment used for all WASM-side allocations.  Must be large enough for
/// `WasmResult` (two `u32`s, align 4) as well as arbitrary byte buffers.
const ALLOC_ALIGN: usize = 8;
/// Version of the allocation and JSON response ABI.
pub const ABI_VERSION: u32 = 1;
const COMPILER_NAME: &str = "deka";
const SOURCE_COMMIT: &str = match option_env!("DEKA_SOURCE_COMMIT") {
    Some(commit) => commit,
    None => "unknown",
};

/// Result descriptor returned by `deka_compiler_compile`. The browser shim
/// reads UTF-8 JSON from `ptr`/`len`, then frees the whole allocation with
/// `deka_compiler_free(result_ptr, size_of::<WasmResult>() + result.len)`.
#[repr(C)]
pub struct WasmResult {
    pub ptr: u32,
    pub len: u32,
}

/// Allocate a buffer of `size` bytes in WASM memory.
#[unsafe(no_mangle)]
pub extern "C" fn deka_compiler_alloc(size: u32) -> *mut u8 {
    let layout = Layout::from_size_align(size as usize, ALLOC_ALIGN).expect("invalid alloc size");
    // SAFETY: layout has non-zero size.
    unsafe { alloc(layout) }
}

/// Free a buffer previously returned by `deka_compiler_alloc`.
///
/// # Safety
///
/// `ptr` must be null or point to a buffer returned by `deka_compiler_alloc`
/// with exactly the supplied `size`; passing any other allocation is undefined.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deka_compiler_free(ptr: *mut u8, size: u32) {
    if ptr.is_null() {
        return;
    }
    let layout = Layout::from_size_align(size as usize, ALLOC_ALIGN).expect("invalid free size");
    // SAFETY: ptr must have been allocated by deka_compiler_alloc with the same size/alignment.
    unsafe { dealloc(ptr, layout) }
}

/// Compile DekaScript source using `mode` (`auto` or `deka`) and return a
/// JSON-encoded `WasmResult`.
///
/// # Safety
/// Non-empty pointer/length pairs must point to valid, immutable UTF-8 buffers
/// in WASM memory. Invalid request text produces a structured diagnostic.
#[unsafe(no_mangle)]
pub extern "C" fn deka_compiler_compile(
    source_ptr: *const u8,
    source_len: u32,
    filename_ptr: *const u8,
    filename_len: u32,
    mode_ptr: *const u8,
    mode_len: u32,
) -> *mut WasmResult {
    let source = read_utf8(source_ptr, source_len, "source");
    let filename = read_utf8(filename_ptr, filename_len, "filename");
    let mode = read_utf8(mode_ptr, mode_len, "mode");
    let json = match (source, filename, mode) {
        (Ok(source), Ok(filename), Ok(mode)) => compile_request(source, filename, mode),
        (source, filename, mode) => request_error(
            filename.unwrap_or("<unknown>"),
            source
                .err()
                .or(filename.err())
                .or(mode.err())
                .unwrap_or("invalid request"),
        ),
    };
    box_result(&json)
}

/// Return static compiler metadata without compiling a source file.
#[unsafe(no_mangle)]
pub extern "C" fn deka_compiler_metadata() -> *mut WasmResult {
    box_result(&json(&CompilerMetadata::current()))
}

fn read_utf8<'a>(ptr: *const u8, len: u32, label: &'static str) -> Result<&'a str, &'static str> {
    if len == 0 {
        return Ok("");
    }
    if ptr.is_null() {
        return Err(match label {
            "source" => "source pointer is null",
            "filename" => "filename pointer is null",
            _ => "mode pointer is null",
        });
    }
    // SAFETY: non-empty buffers have been checked for a non-null pointer; the
    // ABI contract requires the caller to provide a valid readable range.
    let bytes = unsafe { slice::from_raw_parts(ptr, len as usize) };
    str::from_utf8(bytes).map_err(|_| match label {
        "source" => "source is not valid UTF-8",
        "filename" => "filename is not valid UTF-8",
        _ => "mode is not valid UTF-8",
    })
}

#[derive(Serialize)]
struct CompileResponse<'a> {
    abi_version: u32,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<CompileOutput<'a>>,
    diagnostics: Vec<Diagnostic>,
    metadata: CompileMetadata<'a>,
}

#[derive(Serialize)]
struct CompileOutput<'a> {
    code: &'a str,
}

#[derive(Serialize)]
struct CompileMetadata<'a> {
    filename: &'a str,
    language: &'a str,
    compiler: CompilerMetadata,
}

#[derive(Serialize)]
struct CompilerMetadata {
    name: &'static str,
    version: &'static str,
    source_commit: &'static str,
}

impl CompilerMetadata {
    const fn current() -> Self {
        Self {
            name: COMPILER_NAME,
            version: env!("CARGO_PKG_VERSION"),
            source_commit: SOURCE_COMMIT,
        }
    }
}

#[derive(Serialize)]
struct Diagnostic {
    severity: &'static str,
    code: String,
    message: String,
    filename: String,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<String>,
}

fn compile_request(source: &str, filename: &str, requested_mode: &str) -> String {
    let mode = match resolve_mode(filename, requested_mode) {
        Ok(mode) => mode,
        Err(message) => return request_error(filename, message),
    };
    let arena = Bump::new();
    let result = compile_deka(source, filename, &arena);

    let mut diagnostics = result
        .errors
        .iter()
        .map(|error| diagnostic_from_error(error, filename))
        .collect::<Vec<_>>();
    diagnostics.extend(
        result
            .warnings
            .iter()
            .map(|warning| diagnostic_from_warning(warning, filename)),
    );
    let output = if result.errors.is_empty() {
        result.ast.as_ref().map(|program| {
            let meta = phpx_js::parse_source_module_meta(source);
            match phpx_js::emit_js_from_ast_with_warnings(program, source.as_bytes(), meta) {
                Ok((code, warnings)) => {
                    diagnostics.extend(warnings.into_iter().map(|message| Diagnostic {
                        severity: "warning",
                        code: "emitter".to_string(),
                        message,
                        filename: filename.to_string(),
                        start_line: 1,
                        start_column: 1,
                        end_line: 1,
                        end_column: 1,
                        help: None,
                    }));
                    code
                }
                Err(message) => {
                    diagnostics.push(internal_diagnostic(filename, message));
                    String::new()
                }
            }
        })
    } else {
        None
    };
    let ok = output.as_ref().is_some_and(|code| !code.is_empty())
        && !diagnostics.iter().any(|d| d.severity == "error");
    let response = CompileResponse {
        abi_version: ABI_VERSION,
        ok,
        output: output
            .as_ref()
            .filter(|_| ok)
            .map(|code| CompileOutput { code }),
        diagnostics,
        metadata: CompileMetadata {
            filename,
            language: mode,
            compiler: CompilerMetadata::current(),
        },
    };
    json(&response)
}

fn resolve_mode<'a>(filename: &str, mode: &'a str) -> Result<&'a str, &'static str> {
    if !filename.ends_with(".ds") {
        return Err("Deka browser compiler only accepts .ds source files");
    }
    match mode {
        "" | "auto" | "deka" => Ok("deka"),
        _ => Err("unsupported language mode; supported modes are `auto` and `deka`"),
    }
}

fn diagnostic_from_error(error: &ValidationError, filename: &str) -> Diagnostic {
    Diagnostic {
        severity: severity_label(error.severity),
        code: error.kind.as_str().to_string(),
        message: error.message.clone(),
        filename: filename.to_string(),
        start_line: error.line,
        start_column: error.column,
        end_line: error.line,
        end_column: error.column.saturating_add(error.underline_length.max(1)),
        help: (!error.help_text.trim().is_empty()).then(|| error.help_text.clone()),
    }
}

fn diagnostic_from_warning(warning: &ValidationWarning, filename: &str) -> Diagnostic {
    Diagnostic {
        severity: severity_label(warning.severity),
        code: warning.kind.as_str().to_string(),
        message: warning.message.clone(),
        filename: filename.to_string(),
        start_line: warning.line,
        start_column: warning.column,
        end_line: warning.line,
        end_column: warning
            .column
            .saturating_add(warning.underline_length.max(1)),
        help: (!warning.help_text.trim().is_empty()).then(|| warning.help_text.clone()),
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn internal_diagnostic(filename: &str, message: String) -> Diagnostic {
    Diagnostic {
        severity: "error",
        code: "emitter".to_string(),
        message,
        filename: filename.to_string(),
        start_line: 1,
        start_column: 1,
        end_line: 1,
        end_column: 1,
        help: None,
    }
}

fn request_error(filename: &str, message: &str) -> String {
    json(&CompileResponse {
        abi_version: ABI_VERSION,
        ok: false,
        output: None,
        diagnostics: vec![internal_diagnostic(filename, message.to_string())],
        metadata: CompileMetadata {
            filename,
            language: "unknown",
            compiler: CompilerMetadata::current(),
        },
    })
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{\"abi_version\":1,\"ok\":false,\"diagnostics\":[{\"severity\":\"error\",\"code\":\"serialization\",\"message\":\"failed to serialize compiler response\",\"filename\":\"<unknown>\",\"start_line\":1,\"start_column\":1,\"end_line\":1,\"end_column\":1}],\"metadata\":{\"filename\":\"<unknown>\",\"language\":\"unknown\",\"compiler\":{\"name\":\"deka\",\"version\":\"unknown\",\"source_commit\":\"unknown\"}}}".to_string())
}

/// Allocate a single contiguous block containing a `WasmResult` header followed
/// by the JSON payload, then return a pointer to the header.
fn box_result(json: &str) -> *mut WasmResult {
    let header_size = std::mem::size_of::<WasmResult>();
    let total_size = header_size + json.len();
    let base = deka_compiler_alloc(total_size as u32);
    if base.is_null() {
        return ptr::null_mut();
    }

    let result_ptr = base.cast::<WasmResult>();
    let json_ptr = unsafe { base.add(header_size) };

    unsafe {
        ptr::copy_nonoverlapping(json.as_ptr(), json_ptr, json.len());
        (*result_ptr).ptr = json_ptr as u32;
        (*result_ptr).len = json.len() as u32;
    }

    result_ptr
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn deka_mode_compiles_a_ds_fixture_with_structured_metadata() {
        let response: Value =
            serde_json::from_str(&compile_request("const answer = 42;", "lesson.ds", "auto"))
                .expect("response JSON");

        assert_eq!(response["abi_version"], ABI_VERSION);
        assert_eq!(response["ok"], true);
        assert_eq!(response["metadata"]["language"], "deka");
        assert_eq!(response["metadata"]["filename"], "lesson.ds");
        assert!(
            response["output"]["code"]
                .as_str()
                .is_some_and(|code| code.contains("const answer = 42"))
        );
        assert_eq!(response["diagnostics"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn rejects_phpx_mode_and_filename_without_fallback() {
        let filename_response: Value = serde_json::from_str(&compile_request(
            "function greeting($name: string): string { return $name; }",
            "legacy.phpx",
            "phpx",
        ))
        .expect("response JSON");

        assert_eq!(filename_response["ok"], false);
        assert_eq!(filename_response["metadata"]["language"], "unknown");
        assert!(
            filename_response["diagnostics"][0]["message"]
                .as_str()
                .is_some_and(|message| message.contains("only accepts .ds"))
        );

        let mode_response: Value =
            serde_json::from_str(&compile_request("const answer = 42;", "lesson.ds", "phpx"))
                .expect("response JSON");
        assert_eq!(mode_response["ok"], false);
        assert!(
            mode_response["diagnostics"][0]["message"]
                .as_str()
                .is_some_and(|message| message.contains("supported modes are `auto` and `deka`"))
        );
    }

    #[test]
    fn diagnostics_are_monaco_ready_and_native_parity_is_stable() {
        let source = "function broken(";
        let native: Value = serde_json::from_str(&compile_request(source, "broken.ds", "deka"))
            .expect("native response JSON");
        let wasm_abi: Value = serde_json::from_str(&compile_request(source, "broken.ds", "auto"))
            .expect("WASM ABI response JSON");

        assert_eq!(native["ok"], false);
        assert_eq!(native["diagnostics"], wasm_abi["diagnostics"]);
        let diagnostic = &native["diagnostics"][0];
        assert_eq!(diagnostic["severity"], "error");
        assert_eq!(diagnostic["filename"], "broken.ds");
        assert!(diagnostic["start_line"].as_u64().is_some());
        assert!(diagnostic["start_column"].as_u64().is_some());
    }

    #[test]
    fn mode_and_filename_errors_are_structured() {
        let response: Value = serde_json::from_str(&compile_request("", "lesson.txt", "auto"))
            .expect("response JSON");
        assert_eq!(response["ok"], false);
        assert_eq!(response["diagnostics"][0]["code"], "emitter");
        assert_eq!(response["metadata"]["filename"], "lesson.txt");
    }

    #[test]
    fn deka_tour_surface_compiles_with_native_abi_contract() {
        let cases = [
            (
                "typed functions",
                "function add(left: number, right: number): number { return left + right; } console.log(add(20, 22));",
            ),
            ("list literal", "const parts = [\"north\", \"star\"];"),
            (
                "list indexing",
                "const parts = [\"north\", \"star\"]; const first = parts[0];",
            ),
            (
                "object literal and property access",
                "const parts = [\"north\", \"star\"]; const first = parts[0]; const label = { first: first, count: parts.length };",
            ),
            (
                "lists objects and indexing",
                "const parts = [\"north\", \"star\"]; const first = parts[0]; const label = { first: first, count: parts.length }; console.log(`${label.first}:${label.count}`);",
            ),
        ];

        for (name, source) in cases {
            let response: Value = serde_json::from_str(&compile_request(source, "tour.ds", "deka"))
                .unwrap_or_else(|error| panic!("{name}: invalid response JSON: {error}"));
            assert_eq!(response["ok"], true, "{name}: {response}");
            assert!(
                response["output"]["code"].as_str().is_some(),
                "{name}: {response}"
            );
        }

        let sigil: Value =
            serde_json::from_str(&compile_request("const $value = 1;", "tour.ds", "deka"))
                .expect("sigil response JSON");
        assert_eq!(sigil["ok"], false, "{sigil}");
        assert!(
            sigil["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                    diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("bare identifiers"))
                })),
            "sigil diagnostic must explain the DS binding contract: {sigil}"
        );
    }
}
