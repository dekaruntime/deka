#![allow(clippy::all)]

pub mod analytics;
pub mod api;
mod debug;
mod fast;
mod listener;
pub mod rate_limit;
mod router;
mod server;
pub mod utility_css;
pub mod websocket;

pub mod unix;

pub use router::app_router;
pub use server::serve_http;
