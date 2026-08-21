#![allow(clippy::all)]

pub mod config;
pub mod dispatch;
pub mod engine;
pub mod envelope;
pub mod introspect_archive;

use std::sync::Arc;

use pool::HandlerKey;

pub use dispatch::{execute_request, execute_request_parts, execute_request_value};
pub use engine::{RuntimeEngine, engine, set_engine};
pub use envelope::{RequestEnvelope, ResponseEnvelope};
pub use introspect_archive::IntrospectArchive;
pub use runtime_core::storefront_envelope::{StorefrontRequest, StorefrontResponse};

pub struct RuntimeState {
    pub engine: Arc<engine::RuntimeEngine>,
    pub handler_code: String,
    pub handler_entry: Option<String>,
    pub handler_key: HandlerKey,
    pub dev_mode: bool,
    pub perf_mode: bool,
    pub perf_request_value: serde_json::Value,
}
