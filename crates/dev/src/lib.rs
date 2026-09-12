#![allow(clippy::all)]

//! `deka dev` / `deka serve --dev` execution path.
//!
//! Watch, HMR, the listen banner, the isolated compiler cache, and build-slot
//! invalidation live here. Production `deka serve` stays in `runtime` and does
//! not call this crate.

mod banner;
pub mod build_watch;
mod serve;
mod watch;

pub use banner::{announce_listen, ensure_compiler_cache, prepare, print_banner};
pub use serve::{serve, serve_with_dsc};
