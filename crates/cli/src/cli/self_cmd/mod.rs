use core::{CommandSpec, Context, ParamSpec, Registry, SubcommandSpec};

mod fetch;
pub mod monitor;
mod pairing;
mod targets;
mod test;
pub mod update;

const FETCH: SubcommandSpec = SubcommandSpec {
    name: "fetch",
    summary: "fetch a pinned content checkout (testsuite, tour)",
    aliases: &[],
    handler: fetch::cmd,
};

const MONITOR: SubcommandSpec = SubcommandSpec {
    name: "monitor",
    summary: "run the long-running self-update daemon",
    aliases: &[],
    handler: monitor::cmd,
};

const TEST: SubcommandSpec = SubcommandSpec {
    name: "test",
    summary: "run internal deka compatibility and content tests",
    aliases: &[],
    handler: test::cmd,
};

const UPDATE: SubcommandSpec = SubcommandSpec {
    name: "update",
    summary: "update deka components",
    aliases: &[],
    handler: update::cmd,
};

const SUBCOMMANDS: &[SubcommandSpec] = &[FETCH, MONITOR, TEST, UPDATE];

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
    // `self test` overrides: local checkouts to pair, plus runner passthrough.
    // Params are registered globally (the argv parser is command-agnostic);
    // only `self test` reads them.
    for param in [
        ParamSpec {
            name: "--deka",
            description: "self test: pair a local deka checkout or binary",
        },
        ParamSpec {
            name: "--dsc",
            description: "self test: pair a local dsc checkout or binary",
        },
        ParamSpec {
            name: "--filter",
            description: "self test: only run fixtures or lessons matching a substring",
        },
        ParamSpec {
            name: "-f",
            description: "self test: only run fixtures or lessons matching a substring",
        },
        ParamSpec {
            name: "--jobs",
            description: "self test: parallel native runs for the testsuite gate",
        },
    ] {
        registry.add_param(param);
    }
    registry.add_flag(core::FlagSpec {
        name: "--list",
        aliases: &["-l"],
        description: "self test: list fixtures or lessons without running them",
    });
}

fn cmd(_context: &Context) {}
