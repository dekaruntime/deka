//! Experimental PHPX-derived stack VM. No production backend is changed.
mod bytecode;
#[cfg(feature = "compiler")]
pub mod compiler;
#[cfg(feature = "host")]
pub mod demo;
mod heap;
mod host;
mod machine;
mod stack;
pub use bytecode::*;
pub use heap::HeapStats;
pub use host::*;
pub use machine::Vm;
pub type Result<T> = std::result::Result<T, String>;

#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "v8-control")]
#[cfg(feature = "ui")]
pub mod v8_control;
