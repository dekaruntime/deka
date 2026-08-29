use core::{CommandSpec, Context, Registry, SubcommandSpec};
use stdio::error;

#[cfg(feature = "lsp")]
use super::generate::cmd_generate;
use super::migrate::{cmd_flush, cmd_info, cmd_migrate};

#[cfg(feature = "lsp")]
const GENERATE: SubcommandSpec = SubcommandSpec {
    name: "generate",
    summary: "generate db client and migration artifacts from DekaScript struct models",
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

#[cfg(feature = "lsp")]
const SUBCOMMANDS: &[SubcommandSpec] = &[GENERATE, MIGRATE, INFO, FLUSH];
#[cfg(not(feature = "lsp"))]
const SUBCOMMANDS: &[SubcommandSpec] = &[MIGRATE, INFO, FLUSH];

const COMMAND: CommandSpec = CommandSpec {
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
        "missing subcommand. use: deka db generate|migrate|info|flush",
    );
}
