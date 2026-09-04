use core::{CommandSpec, Context, FlagSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    name: "serve",
    category: "runtime",
    summary: "serve a handler or directory",
    aliases: &["start"],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--dev",
        aliases: &[],
        description: "enable development runtime mode (watch + hmr scaffolding)",
    });
}

pub fn cmd(context: &Context) {
    runtime::serve(context);
}
