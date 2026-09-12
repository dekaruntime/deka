#![allow(clippy::all)]

pub mod config;
pub mod dispatch;
pub mod engine;
pub mod envelope;
pub mod introspect_archive;

use std::path::PathBuf;
use std::sync::Arc;

use pool::HandlerKey;

pub use dispatch::{execute_request, execute_request_parts, execute_request_value};
pub use engine::{RuntimeEngine, engine, set_engine};
pub use envelope::{RequestEnvelope, ResponseEnvelope};
pub use introspect_archive::IntrospectArchive;
pub use serve::request_envelope::{StorefrontRequest, StorefrontResponse};

pub struct RuntimeState {
    pub engine: Arc<engine::RuntimeEngine>,
    pub handler_code: String,
    pub handler_entry: Option<String>,
    /// App-router projects expose this directory at the request root.
    pub public_dir: Option<PathBuf>,
    /// Present only for a verified authored artifact. The HTTP dispatcher uses
    /// it to authenticate client bytes lazily before returning them.
    pub artifact_manifest: Option<runtime_core::dist::ArtifactManifestV2>,
    pub handler_key: HandlerKey,
    pub dev_mode: bool,
    pub perf_mode: bool,
    pub perf_request_value: serde_json::Value,
    /// Resolved security policy every dispatched request executes under.
    /// Mandatory since deka#801: enforcement errors on a missing context,
    /// never falling back to the process environment or a default.
    pub security: pool::ExecutionSecurity,
}
