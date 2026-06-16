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
mod stdlib_prelude;
mod types;

pub use compiler::{
    compile_phpx_source_to_js, emit_js_from_ast, emit_js_from_ast_with_warnings,
    emit_js_scaffold_with_reason,
};
pub use metadata::parse_source_module_meta;
pub use stdlib_prelude::build_stdlib_prelude;
pub use types::{ImportDecl, ImportSpec, SourceModuleMeta};

#[cfg(test)]
mod tests;
