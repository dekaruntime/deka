pub mod data_envelope;
pub mod env;
pub mod framework;
pub mod handler;
pub mod module_spec;
pub mod modules;
pub mod platform_env;
pub mod process;
pub mod security;
pub mod seam;
pub mod security_policy;
pub mod storefront_envelope;
pub mod validation;

pub use security_policy::merge_policy_with_cli_manifest_net_env;

/// Marker prefixed to validation error messages that are propagated from the
/// compiler through the isolate loader so the runtime can print them without
/// its own "Run failed:" wrapper.
pub const DEKA_VALIDATION_ERROR_MARKER: &str = "DEKA_VALIDATION_ERROR:";
