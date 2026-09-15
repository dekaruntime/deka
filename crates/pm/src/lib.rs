#![allow(clippy::all)]

pub mod init;
#[cfg(not(target_arch = "wasm32"))]
mod cli_install;
#[cfg(not(target_arch = "wasm32"))]
mod cli_link;
#[cfg(not(target_arch = "wasm32"))]
mod cli_pkg;
#[cfg(not(target_arch = "wasm32"))]
mod cli_release;
#[cfg(not(target_arch = "wasm32"))]
mod cli_summon;

pub mod cache;
pub mod grants;
pub mod install;
#[cfg(test)]
mod install_fixture_registry;
pub mod links;
pub mod lock;
pub mod payload;
mod recovery_report;
pub mod registry;
#[cfg(feature = "self-update")]
pub mod releases;
pub mod spec;
pub mod version_range;

pub mod registry_integrity;

pub use install::run_install;
pub use links::{link_package_at, unlink_package_at};
pub use payload::InstallPayload;

pub mod summon;

use deka_cli_core::Registry;

pub fn register(registry: &mut Registry) {
    register_init(registry);
    #[cfg(not(target_arch = "wasm32"))]
    {
        register_install(registry);
        register_link(registry);
        register_summon(registry);
        register_pkg(registry);
        register_release(registry);
    }
}

pub fn register_init(registry: &mut Registry) {
    init::register(registry);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn register_install(registry: &mut Registry) {
    cli_install::register(registry);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn register_link(registry: &mut Registry) {
    cli_link::register(registry);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn register_summon(registry: &mut Registry) {
    cli_summon::register(registry);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn register_pkg(registry: &mut Registry) {
    cli_pkg::register(registry);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn register_release(registry: &mut Registry) {
    cli_release::register(registry);
}
