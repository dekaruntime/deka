#![allow(clippy::all)]

pub mod cache;
pub mod install;
pub mod lock;
pub mod payload;
pub mod spec;

pub mod registry_integrity;

pub use install::run_install;
pub use payload::InstallPayload;
