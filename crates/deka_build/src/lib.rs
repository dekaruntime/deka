#![allow(clippy::all, dead_code, unused_variables, unused_assignments, unused_imports)]

mod command;
pub mod dsc;
pub mod project;
pub mod publish;
mod server_entries;
mod server_graph;
mod single_file;
pub mod slots;

pub use command::register;
pub use slots::{
    BuildSlotRefresh, BuildSlotRefreshRequest, ensure_dev_build_slots, make_dev_refresh_callback,
};
