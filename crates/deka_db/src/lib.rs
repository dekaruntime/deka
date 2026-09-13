#![allow(clippy::all, dead_code, unused_variables, unused_assignments)]

use deka_cli_core::Registry;

#[cfg(not(target_arch = "wasm32"))]
mod commands;
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
mod migrate;
#[cfg(not(target_arch = "wasm32"))]
mod model;

#[cfg(target_arch = "wasm32")]
mod wasm;

pub fn register(registry: &mut Registry) {
    #[cfg(not(target_arch = "wasm32"))]
    commands::register(registry);
    #[cfg(target_arch = "wasm32")]
    wasm::register(registry);
}
