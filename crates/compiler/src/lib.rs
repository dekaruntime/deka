pub mod dsc;

mod check;
mod fmt;
mod transpile;

#[cfg(feature = "lsp")]
mod lsp;

use core::Registry;

pub fn register(registry: &mut Registry) {
    register_check(registry);
    register_fmt(registry);
    register_transpile(registry);
    #[cfg(feature = "lsp")]
    register_lsp(registry);
}

pub fn register_check(registry: &mut Registry) {
    check::register(registry);
}

pub fn register_fmt(registry: &mut Registry) {
    fmt::register(registry);
}

pub fn register_transpile(registry: &mut Registry) {
    transpile::register(registry);
}

#[cfg(feature = "lsp")]
pub fn register_lsp(registry: &mut Registry) {
    lsp::register(registry);
}
