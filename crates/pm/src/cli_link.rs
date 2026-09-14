use deka_cli_core::{CommandSpec, Context, Registry};
use crate::{link_package_at, unlink_package_at};
use std::path::{Path, PathBuf};
use stdio;

const LINK_COMMAND: CommandSpec = CommandSpec {
    owner: "pm",
    name: "link",
    category: "package",
    summary: "link a local package working tree into this project",
    aliases: &[],
    subcommands: &[],
    handler: cmd_link,
};

const UNLINK_COMMAND: CommandSpec = CommandSpec {
    owner: "pm",
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
    // Argument-shape checks (deka#1010: usage error, exit 2) run before the
    // closure below, which owns everything that can only fail once we
    // actually try to link (runtime failure, exit 1).
    if context.args.positionals.len() != 1 {
        stdio::error("link", "usage: deka link <package-directory>");
        std::process::exit(2);
    }

    let result = (|| {
        let package_dir = &context.args.positionals[0];
        let project_dir = discover_project_root()?;
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
    if context.args.positionals.len() != 1 {
        stdio::error("unlink", "usage: deka unlink <package-name>");
        std::process::exit(2);
    }

    let result = (|| {
        let package = &context.args.positionals[0];
        let project_dir = discover_project_root()?;
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

fn discover_project_root() -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir()
        .map_err(|err| anyhow::anyhow!("failed to resolve project directory: {err}"))?;
    discover_project_root_from(&cwd)
}

fn discover_project_root_from(start: &Path) -> anyhow::Result<PathBuf> {
    let mut current = start;
    loop {
        if current.join("deka.json").is_file() {
            return std::fs::canonicalize(current).map_err(|err| {
                anyhow::anyhow!(
                    "failed to resolve project root {}: {err}",
                    current.display()
                )
            });
        }
        current = current.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "could not find deka.json for project command from {}",
                start.display()
            )
        })?;
    }
}

#[cfg(test)]
mod tests {
    use super::discover_project_root_from;
    use std::fs;

    #[test]
    fn discovers_root_from_nested_directory() {
        let project = tempfile::tempdir().unwrap();
        let nested = project.path().join("src").join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(project.path().join("deka.json"), "{}\n").unwrap();

        let discovered = discover_project_root_from(&nested).unwrap();

        assert_eq!(discovered, project.path().canonicalize().unwrap());
    }
}
