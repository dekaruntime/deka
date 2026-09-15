use dcore::{CommandSpec, Context, ParamSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "compiler",
    name: "fmt",
    category: "project",
    summary: "format DekaScript source or emitted JavaScript (execs dsc)",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_param(ParamSpec {
        name: "--lang",
        description: "language to format: ds or js (default: ds)",
    });
    registry.add_flag(dcore::FlagSpec {
        name: "--check",
        aliases: &[],
        description: "exit non-zero if files would change",
    });
    registry.add_flag(dcore::FlagSpec {
        name: "--stdin",
        aliases: &[],
        description: "read source from stdin instead of a file",
    });
}

pub fn cmd(_context: &Context) {
    crate::dsc::exec_if_present();
}
