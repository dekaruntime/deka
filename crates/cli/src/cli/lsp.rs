use core::{CommandSpec, Context, FlagSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    name: "lsp",
    category: "tooling",
    summary: "run the DekaScript language server (forwards to dsc)",
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
    // The language server ships in dsc; deka only forwards. `execute` already
    // execs dsc for single-word commands, this covers direct handler calls.
    crate::dsc::exec_if_present();
    std::process::exit(1);
}
