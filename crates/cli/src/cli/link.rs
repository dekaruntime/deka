use core::{CommandSpec, Context, Registry};
use pm::{link_package_at, unlink_package_at};
use std::path::PathBuf;
use stdio;

const LINK_COMMAND: CommandSpec = CommandSpec {
    name: "link",
    category: "package",
    summary: "link a local package working tree into this project",
    aliases: &[],
    subcommands: &[],
    handler: cmd_link,
};

const UNLINK_COMMAND: CommandSpec = CommandSpec {
    name: "unlink",
    category: "package",
    summary: "remove a local package link without touching its target",
    aliases: &[],
    subcommands: &[],
    handler: cmd_unlink,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(LINK_COMMAND);
    registry.add_command(UNLINK_COMMAND);
}

pub fn cmd_link(context: &Context) {
    let result = (|| {
        let package_dir = context
            .args
            .positionals
            .first()
            .ok_or_else(|| anyhow::anyhow!("usage: deka link <package-directory>"))?;
        if context.args.positionals.len() != 1 {
            anyhow::bail!("usage: deka link <package-directory>");
        }
        let project_dir = std::env::current_dir()
            .map_err(|err| anyhow::anyhow!("failed to resolve project directory: {err}"))?;
        let name = link_package_at(&project_dir, &PathBuf::from(package_dir))?;
        Ok::<_, anyhow::Error>(name)
    })();

    match result {
        Ok(name) => stdio::success(&format!("linked {name}")),
        Err(error) => {
            stdio::error("link", &error.to_string());
            std::process::exit(1);
        }
    }
}

pub fn cmd_unlink(context: &Context) {
    let result = (|| {
        let package = context
            .args
            .positionals
            .first()
            .ok_or_else(|| anyhow::anyhow!("usage: deka unlink <package-name>"))?;
        if context.args.positionals.len() != 1 {
            anyhow::bail!("usage: deka unlink <package-name>");
        }
        let project_dir = std::env::current_dir()
            .map_err(|err| anyhow::anyhow!("failed to resolve project directory: {err}"))?;
        let target = unlink_package_at(&project_dir, package)?;
        Ok::<_, anyhow::Error>((package.clone(), target))
    })();

    match result {
        Ok((package, _target)) => stdio::success(&format!("unlinked {package}")),
        Err(error) => {
            stdio::error("unlink", &error.to_string());
            std::process::exit(1);
        }
    }
}
