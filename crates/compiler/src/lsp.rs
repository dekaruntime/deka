use core::{CommandSpec, Context, FlagSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "compiler",
    name: "lsp",
    category: "tooling",
    summary: "run the DekaScript language server",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--stdio",
        aliases: &[],
        description: "run the language server over stdio",
    });
}

pub fn cmd(_context: &Context) {
    crate::dsc::exec_if_present();
}
