#![allow(clippy::all)]

pub mod config;
mod fast;
mod listener;
pub mod rate_limit;
pub mod react_refresh;
mod router;
mod server;
pub mod utility_css;
pub mod websocket;

pub mod unix;

#[cfg(test)]
mod websocket_hmr_tests;

pub use config::HttpConfig;
pub use router::app_router;
pub use server::serve_http;
