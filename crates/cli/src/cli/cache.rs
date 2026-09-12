//! `deka cache` — manage the user-global cache (deka#765).
//!
//! Loose-file runs materialize compiled artifacts under the user cache; a
//! user-global cache grows without bound, so eviction runs automatically on
//! materialize and `deka cache clear` empties it explicitly.

use core::{CommandSpec, Context, Registry};

use crate::cli::user_cache;

const CLEAR: core::SubcommandSpec = core::SubcommandSpec {
    name: "clear",
    summary: "remove all loose-file cache entries",
    aliases: &[],
    handler: clear_cmd,
};

const COMMAND: CommandSpec = CommandSpec {
    owner: "",
    name: "cache",
    category: "runtime",
    summary: "manage the user-global cache",
    aliases: &[],
    subcommands: &[CLEAR],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

pub fn cmd(_context: &Context) {
    stdio::error("cache", "expected a subcommand: clear");
    std::process::exit(2);
}

fn clear_cmd(_context: &Context) {
    match clear() {
        Ok(()) => stdio::success("cache cleared"),
        Err(err) => {
            stdio::error("cache", &err);
            std::process::exit(1);
        }
    }
}

fn clear() -> Result<(), String> {
    let root = user_cache::resolve_user_cache_root()?;
    user_cache::clear_loose_cache(&root)
}
