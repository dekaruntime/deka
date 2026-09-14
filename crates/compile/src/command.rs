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
    registry.add_param(core::ParamSpec {
        name: "--outfile",
        description: "compiled executable output path (default: deka-app)",
    });
    registry.add_flag(core::FlagSpec {
        name: "--desktop",
        aliases: &[],
        description: "package a web project as a desktop app (React in webview)",
    });
}

pub fn cmd(context: &Context) {
    crate::run(context);
}
