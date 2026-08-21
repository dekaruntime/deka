use core::{CommandSpec, Context, Registry, SubcommandSpec};

mod monitor;
mod test;
mod update;

const TEST: SubcommandSpec = SubcommandSpec {
    name: "test",
    summary: "run internal deka compatibility tests",
    aliases: &[],
    handler: test::cmd,
};

const MONITOR: SubcommandSpec = SubcommandSpec {
    name: "monitor",
    summary: "run the long-running self-update daemon",
    aliases: &[],
    handler: monitor::cmd,
};

const UPDATE: SubcommandSpec = SubcommandSpec {
    name: "update",
    summary: "update deka components",
    aliases: &[],
    handler: update::cmd,
};

const SUBCOMMANDS: &[SubcommandSpec] = &[TEST, MONITOR, UPDATE];

const COMMAND: CommandSpec = CommandSpec {
    name: "self",
    category: "internal",
    summary: "internal deka maintenance commands",
    aliases: &[],
    subcommands: SUBCOMMANDS,
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

fn cmd(_context: &Context) {}
