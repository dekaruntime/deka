#![allow(clippy::all, dead_code, unused_variables, unused_assignments)]

pub mod auth;
pub mod auth_store;
pub mod publish;

use deka_cli_core::Registry;

pub fn register(registry: &mut Registry) {
    auth::register(registry);
    publish::register(registry);
}
