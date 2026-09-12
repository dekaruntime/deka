use core::{CommandSpec, Context, ParamSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "",
    name: "transpile",
    category: "project",
    summary: "emit JavaScript from a .ds file or directory (execs dsc)",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(core::FlagSpec {
        name: "--preserve",
        aliases: &[],
        description: "preserve a directory's module tree (default for directories)",
    });
    registry.add_flag(core::FlagSpec {
        name: "--bundle",
        aliases: &[],
        description: "emit one resolved JavaScript module graph",
    });
    registry.add_flag(core::FlagSpec {
        name: "--treeshake",
        aliases: &[],
        description: "apply JavaScript optimization to emitted modules",
    });
    registry.add_flag(core::FlagSpec {
        name: "--client",
        aliases: &[],
        description: "treat the entry as a client bundle (ui/server is a build failure)",
    });
    registry.add_param(ParamSpec {
        name: "--out",
        description: "output .js file (file/bundle) or output directory (preserve)",
    });
}

pub fn cmd(_context: &Context) {
    stdio::error(
        "transpile",
        "internal error: deka transpile should have exec'd dsc",
    );
    std::process::exit(1);
}
