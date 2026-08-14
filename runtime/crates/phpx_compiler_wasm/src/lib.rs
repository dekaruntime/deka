//! WASM compiler bridge for PHPX → JavaScript.
//!
//! Exports a small C-style ABI so the browser shim can allocate input strings,
//! invoke compilation, and free the returned JSON buffer.

use std::alloc::{alloc, dealloc, Layout};
use std::{ptr, slice, str};

/// Alignment used for all WASM-side allocations.  Must be large enough for
/// `WasmResult` (two `u32`s, align 4) as well as arbitrary byte buffers.
const ALLOC_ALIGN: usize = 8;

/// Result descriptor returned by `phpx_compile`.  The browser shim reads the
/// UTF-8 JSON from `ptr`/`len` and then frees the whole allocation with
/// `phpx_compile_free(result_ptr, size_of::<WasmResult>() + result.len)`.
#[repr(C)]
pub struct WasmResult {
    pub ptr: u32,
    pub len: u32,
}

/// Allocate a buffer of `size` bytes in the WASM memory.
#[unsafe(no_mangle)]
pub extern "C" fn phpx_compile_alloc(size: u32) -> *mut u8 {
    let layout = Layout::from_size_align(size as usize, ALLOC_ALIGN).expect("invalid alloc size");
    // SAFETY: layout has non-zero size.
    unsafe { alloc(layout) }
}

/// Free a buffer previously returned by `phpx_compile_alloc`.
#[unsafe(no_mangle)]
pub extern "C" fn phpx_compile_free(ptr: *mut u8, size: u32) {
    if ptr.is_null() {
        return;
    }
    let layout = Layout::from_size_align(size as usize, ALLOC_ALIGN).expect("invalid free size");
    // SAFETY: ptr must have been allocated by phpx_compile_alloc with the same size/alignment.
    unsafe { dealloc(ptr, layout) }
}

/// Compile PHPX source to JavaScript and return a JSON-encoded `WasmResult`.
///
/// # Safety
/// `source_ptr`/`source_len` and `filename_ptr`/`filename_len` must point to
/// valid, immutable UTF-8 buffers in WASM memory.
#[unsafe(no_mangle)]
pub extern "C" fn phpx_compile(
    source_ptr: *const u8,
    source_len: u32,
    filename_ptr: *const u8,
    filename_len: u32,
) -> *mut WasmResult {
    // SAFETY: caller guarantees the pointers/lengths are valid.
    let source_bytes = unsafe { slice::from_raw_parts(source_ptr, source_len as usize) };
    let filename_bytes = unsafe { slice::from_raw_parts(filename_ptr, filename_len as usize) };

    let source = match str::from_utf8(source_bytes) {
        Ok(s) => s,
        Err(_) => return box_result(r#"{"ok":false,"error":"source is not valid UTF-8"}"#),
    };
    let filename = match str::from_utf8(filename_bytes) {
        Ok(s) => s,
        Err(_) => return box_result(r#"{"ok":false,"error":"filename is not valid UTF-8"}"#),
    };

    let json = compile(source, filename);
    box_result(&json)
}

fn compile(source: &str, filename: &str) -> String {
    let meta = phpx_js::parse_source_module_meta(source);
    match phpx_js::compile_phpx_source_to_js(source, filename, meta) {
        Ok(js) => {
            let js_json = serde_json::to_string(&js).unwrap_or_else(|_| "\"\"".to_string());
            format!(r#"{{"ok":true,"js":{},"warnings":[]}}"#, js_json)
        }
        Err(err) => {
            let err_json = serde_json::to_string(&err).unwrap_or_else(|_| "\"unknown error\"".to_string());
            format!(r#"{{"ok":false,"error":{}}}"#, err_json)
        }
    }
}

/// Allocate a single contiguous block containing a `WasmResult` header followed
/// by the JSON payload, then return a pointer to the header.
fn box_result(json: &str) -> *mut WasmResult {
    let header_size = std::mem::size_of::<WasmResult>();
    let total_size = header_size + json.len();
    let base = phpx_compile_alloc(total_size as u32);
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
