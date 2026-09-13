#![allow(clippy::all, dead_code, unused_variables, unused_assignments)]

mod command;
mod pipeline_yaml;
mod runner;

pub use command::{register, resolve_gild_endpoint};
