#![allow(clippy::all)]

#[cfg(feature = "json")]
pub use deka_wasm_guest_macros::{deka_export_json, export_json};

#[repr(C)]
pub struct WasmResult {
    pub ptr: u32,
    pub len: u32,
}

/// Allocate a buffer in guest memory for the host to write into.
#[no_mangle]
pub extern "C" fn deka_alloc(size: u32) -> *mut u8 {
    let mut buf = Vec::with_capacity(size as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Free a buffer previously allocated by deka_alloc.
#[no_mangle]
pub extern "C" fn deka_free(ptr: *mut u8, size: u32) {
    if ptr.is_null() || size == 0 {
        return;
    }
    unsafe {
        drop(Vec::from_raw_parts(ptr, size as usize, size as usize));
    }
}

pub fn read_string(ptr: *const u8, len: u32) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    String::from_utf8_lossy(bytes).to_string()
}

pub fn write_result(value: &str) -> *mut WasmResult {
    let bytes = value.as_bytes();
    let ptr = deka_alloc(bytes.len() as u32);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    }
    let result = Box::new(WasmResult {
        ptr: ptr as u32,
        len: bytes.len() as u32,
    });
    Box::into_raw(result)
}
