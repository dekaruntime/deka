#![allow(clippy::all, dead_code, unused_variables, unused_assignments)]

mod command;
pub mod user_cache;

pub use command::register;
pub use user_cache::{
    NOT_A_PROJECT_NOTE, clear_loose_cache, is_loose_source_file, materialize_loose,
    prepare_loose_serve, resolve_user_cache_root, rewrite_context_for_artifact,
};
