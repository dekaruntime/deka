mod commands;
mod config;
#[cfg(feature = "lsp")]
mod generate;
mod migrate;
mod model;

pub use commands::register;
