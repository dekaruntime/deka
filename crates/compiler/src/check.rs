use core::{CommandSpec, Context, FlagSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "",
    name: "check",
    category: "project",
    summary: "validate a DekaScript source file (execs dsc)",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--as-package",
        aliases: &[],
        description: "typecheck a package working tree the way an installed consumer would resolve it",
    });
    registry.add_flag(FlagSpec {
        name: "--single-file",
        aliases: &[],
        description: "typecheck only the requested file without project imports",
    });
}

pub fn cmd(_context: &Context) {
    stdio::error(
        "check",
        "internal error: deka check should have exec'd dsc",
    );
    std::process::exit(1);
}
