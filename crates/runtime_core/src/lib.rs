pub mod data_envelope;
pub mod deka_catalog;
pub mod ds_imports;
pub mod ds_tla;
pub mod dsc;
pub mod entry;
pub mod env;
pub mod dist;
pub mod handler;
pub mod module_spec;
pub mod modules;
pub mod platform_env;
pub mod project_gate;
pub mod process;
pub mod seam;
pub mod storefront_envelope;
pub mod validation;

/// Marker prefixed to validation error messages that are propagated from the
/// compiler through the isolate loader so the runtime can print them without
/// its own "Run failed:" wrapper.
pub const DEKA_VALIDATION_ERROR_MARKER: &str = "DEKA_VALIDATION_ERROR:";
