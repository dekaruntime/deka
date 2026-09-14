use deka_cli_core::{CommandSpec, Context, Registry};
use std::io::{IsTerminal, Write};

pub fn register(registry: &mut Registry) {
    registry.add_command(CommandSpec {
        owner: "pm",
        name: "summon",
        category: "package",
        summary: "fetch, vet and lock a JavaScript URL or JSR package",
        aliases: &[],
        subcommands: &[],
        handler: cmd,
    });
}

fn cmd(context: &Context) {
    // Argument-shape check runs before the closure (deka#1010: usage
    // error, exit 2); everything the closure can fail on is a runtime
    // failure (exit 1).
    let [source] = context.args.positionals.as_slice() else {
        stdio::error(
            "summon",
            "usage: deka summon <url|jsr:@scope/name[@version]> (JavaScript, .tgz or JSR; infer is a later stage)",
        );
        std::process::exit(2);
    };

    let result = (|| {
        let cwd = std::env::current_dir()?;
        let project = cwd
            .ancestors()
            .find(|path| path.join("deka.json").is_file())
            .ok_or_else(|| anyhow::anyhow!("summon requires deka.json; run deka init first"))?;
        crate::summon::summon_at(project, source, |name| {
            if !std::io::stdin().is_terminal() && !context.args.flags.contains_key("--prompt") {
                anyhow::bail!(
                    "summon conflict: @js/{name} already exists; use --prompt to choose a different vendor name"
                );
            }
            eprint!(
                "summon conflict: @js/{name} already exists. New vendor name (blank cancels): "
            );
            std::io::stderr().flush()?;
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            if answer.trim().is_empty() {
                anyhow::bail!("summon cancelled; existing package unchanged");
            }
            Ok(answer.trim().to_string())
        })
    })();
    match result {
        Ok(result) => {
            for warning in result.warnings {
                eprintln!("warning: {warning}");
            }
            stdio::success(&format!(
                "summoned {} (vendored and SHA-256 locked); write your summon block",
                result.spec
            ));
        }
        Err(error) => {
            stdio::error("summon", &format!("{error:#}"));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shim_registers_summon_with_pm_owner() {
        let mut registry = deka_cli_core::Registry::new();
        super::register(&mut registry);
        let command = registry
            .commands()
            .iter()
            .find(|command| command.name == "summon")
            .unwrap();
        assert_eq!(command.owner, "pm");
    }
}
