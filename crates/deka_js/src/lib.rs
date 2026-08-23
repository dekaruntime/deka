#![allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    dead_code,
    unused_variables
)]

#[cfg(test)]
use bumpalo::Bump;

mod compiler;
mod emitter;
mod metadata;
mod types;

pub use compiler::{
    CompileError, CompileOutcome, DEKA_VALIDATION_ERROR_MARKER,
    compile_phpx_source_to_js, compile_phpx_source_to_js_with_warnings,
    compile_phpx_source_to_js_with_warnings_detailed, emit_js_from_ast,
    emit_js_from_ast_with_warnings, emit_js_scaffold_with_reason,
};
pub use metadata::parse_source_module_meta;
pub use types::{ImportDecl, ImportSpec, SourceModuleMeta};

#[cfg(test)]
mod tests;
