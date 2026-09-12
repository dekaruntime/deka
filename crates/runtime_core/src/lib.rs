pub mod data_envelope;
pub mod ds_tla;
pub mod dist;
pub mod platform_env;
pub mod process;

/// Marker prefixed to validation error messages that are propagated from the
/// compiler through the isolate loader so the runtime can print them without
/// its own "Run failed:" wrapper.
pub const DEKA_VALIDATION_ERROR_MARKER: &str = "DEKA_VALIDATION_ERROR:";
