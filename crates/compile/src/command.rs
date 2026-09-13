use core::{CommandSpec, Context, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "compile",
    name: "compile",
    category: "project",
    summary: "compile to single-file executable",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

pub fn cmd(context: &Context) {
    crate::run(context);
}
