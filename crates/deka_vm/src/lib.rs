//! PHPX-derived Rust bytecode VM shared by native Deka and browser previews.
#[cfg(feature = "host")]
pub mod abort;
#[cfg(feature = "host")]
pub mod builtin_crypto;
#[cfg(feature = "host")]
pub mod builtin_http;
mod bytecode;
pub mod bytes;
mod callback;
#[cfg(feature = "host")]
pub mod crypto;
#[cfg(feature = "host")]
mod http_transport;
pub mod text_codec;
pub use callback::{HostCallback, HostContext, HostJob};
#[cfg(feature = "host")]
pub mod blob;
#[cfg(feature = "host")]
pub mod builtin_fs;
pub mod builtin_math;
#[cfg(feature = "host")]
pub mod builtin_time;
#[cfg(feature = "compiler")]
pub mod compiler;
#[cfg(feature = "host")]
pub mod demo;
#[cfg(feature = "host")]
pub mod fetch;
mod heap;
mod host;
pub mod http_body;
pub mod http_headers;
pub mod http_response;
#[cfg(feature = "host")]
pub mod http_server;
mod json;
#[cfg(feature = "host")]
pub mod jwt;
#[cfg(any(feature = "compiler", feature = "host"))]
mod jwt_contract;
pub use json::{JsonField, JsonShape};
mod machine;
#[cfg(feature = "compiler")]
pub mod package;
mod stack;
mod turn;
#[cfg(any(feature = "compiler", feature = "ui"))]
mod ui_contract;
pub use turn::Turn;
#[cfg(feature = "host")]
pub mod tcp;
#[cfg(feature = "host")]
pub mod time;
#[cfg(feature = "host")]
pub mod timers;
#[cfg(feature = "host")]
pub mod tls;
pub use bytecode::*;
pub use heap::HeapStats;
pub use host::*;
pub use machine::Vm;
pub type Result<T> = std::result::Result<T, String>;

#[cfg(feature = "ui")]
pub mod component;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "v8-control")]
#[cfg(feature = "ui")]
pub mod v8_control;

#[cfg(feature = "host")]
pub mod url;

#[cfg(feature = "host")]
pub mod http_request;

#[path = "../../deka_syntax/src/native_brand.rs"]
mod native_brand;
