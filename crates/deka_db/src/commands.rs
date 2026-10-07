use deka_cli_core::{CommandSpec, Context, Registry, SubcommandSpec};
use stdio::error;

use super::migrate::{cmd_flush, cmd_info, cmd_migrate};

const MIGRATE: SubcommandSpec = SubcommandSpec {
    name: "migrate",
    summary: "apply pending db migrations",
    aliases: &[],
    handler: cmd_migrate,
};

const INFO: SubcommandSpec = SubcommandSpec {
    name: "info",
    summary: "show db generation and migration state",
    aliases: &["status"],
    handler: cmd_info,
};

const FLUSH: SubcommandSpec = SubcommandSpec {
    name: "flush",
    summary: "reset database schema (dev only)",
    aliases: &[],
    handler: cmd_flush,
};

const SUBCOMMANDS: &[SubcommandSpec] = &[MIGRATE, INFO, FLUSH];

const COMMAND: CommandSpec = CommandSpec {
    owner: "db",
    name: "db",
    category: "database",
    summary: "database tooling for DekaScript ORM generation and migrations",
    aliases: &[],
    subcommands: SUBCOMMANDS,
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

fn cmd(_context: &Context) {
    error(
        "db",
        "missing subcommand. use: deka db migrate|info|flush",
    );
    // Missing subcommand is a usage error (deka#1010): the CLI's documented
    // convention is exit 2 for usage/parse errors, 1 for runtime failures.
    // This handler previously fell through and returned 0 on the exact
    // failure path it just printed an error for.
    std::process::exit(2);
}
