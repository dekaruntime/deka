//! PHPX-derived Rust bytecode VM shared by native Deka and browser previews.
#[cfg(feature = "host")]
pub mod abort;
#[cfg(feature = "host")]
pub mod builtin_crypto;
mod bytecode;
pub mod bytes;
mod callback;
#[cfg(feature = "host")]
pub mod crypto;
pub mod text_codec;
pub use callback::{HostCallback, HostContext, HostJob};
#[cfg(feature = "host")]
pub mod blob;
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
pub mod http_headers;
pub mod http_response;
mod json;
pub use json::{JsonField, JsonShape};
mod machine;
#[cfg(feature = "compiler")]
pub mod package;
mod stack;
mod turn;
pub use turn::Turn;
#[cfg(feature = "host")]
pub mod time;
#[cfg(feature = "host")]
pub mod timers;
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
