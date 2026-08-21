use core::{CommandSpec, Context, Registry, SubcommandSpec};
use stdio::error;

use super::{
    generate::cmd_generate,
    migrate::{cmd_flush, cmd_info, cmd_migrate},
};

const GENERATE: SubcommandSpec = SubcommandSpec {
    name: "generate",
    summary: "generate db client and migration artifacts from PHPX struct models",
    aliases: &["gen"],
    handler: cmd_generate,
};

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

const SUBCOMMANDS: &[SubcommandSpec] = &[GENERATE, MIGRATE, INFO, FLUSH];

const COMMAND: CommandSpec = CommandSpec {
    name: "db",
    category: "database",
    summary: "database tooling for PHPX ORM generation and migrations",
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
        "missing subcommand. use: deka db generate|migrate|info|flush",
    );
}
