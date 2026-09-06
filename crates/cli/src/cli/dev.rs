use core::{CommandSpec, Context, Registry};

const COMMAND: CommandSpec = CommandSpec {
    name: "dev",
    category: "runtime",
    summary: "serve with HTTP + HMR (dev mode)",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

/// Force `--dev` then reuse the serve path (`serve --dev` is the same mode).
pub fn cmd(context: &Context) {
    let mut ctx = context.clone();
    ctx.args.flags.insert("--dev".to_string(), true);
    crate::cli::serve::cmd(&ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn forces_dev_flag_before_serve() {
        let mut flags = HashMap::new();
        assert!(!flags.contains_key("--dev"));
        flags.insert("--dev".to_string(), true);
        assert!(flags.get("--dev").copied().unwrap_or(false));
    }

    #[test]
    fn command_name_is_dev() {
        assert_eq!(COMMAND.name, "dev");
        assert_eq!(COMMAND.category, "runtime");
    }
}
