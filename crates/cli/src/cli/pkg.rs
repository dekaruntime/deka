use core::{CommandSpec, Context, Registry, SubcommandSpec};

const COMMAND: CommandSpec = CommandSpec {
    name: "pkg",
    category: "package",
    summary: "package operations",
    aliases: &[],
    subcommands: &[
        INSTALL_SUBCOMMAND,
        UPDATE_SUBCOMMAND,
        PUBLISH_SUBCOMMAND,
        RELEASE_SUBCOMMAND,
    ],
    handler: cmd,
};

const INSTALL_SUBCOMMAND: SubcommandSpec = SubcommandSpec {
    name: "install",
    summary: "install package(s)",
    aliases: &["add", "i"],
    handler: crate::cli::install::cmd,
};

const UPDATE_SUBCOMMAND: SubcommandSpec = SubcommandSpec {
    name: "update",
    summary: "update dependencies to latest within semver range",
    aliases: &[],
    handler: crate::cli::install::cmd_update,
};

const PUBLISH_SUBCOMMAND: SubcommandSpec = SubcommandSpec {
    name: "publish",
    summary: "publish package",
    aliases: &[],
    handler: crate::cli::publish::cmd,
};

const RELEASE_SUBCOMMAND: SubcommandSpec = SubcommandSpec {
    name: "release",
    summary: "bump, tag, push, and publish package",
    aliases: &[],
    handler: crate::cli::release::cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

fn cmd(_context: &Context) {
    stdio::log(
        "pkg",
        "available subcommands: install, update, publish, release",
    );
}
