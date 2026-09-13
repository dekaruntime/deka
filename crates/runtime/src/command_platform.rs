use core::{CommandSpec, Context, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "",
    name: "platform",
    category: "runtime",
    summary: "multi-tenant platform server — serves per-tenant DekaScript handlers from tenants/ directory",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

pub fn cmd(context: &Context) {
    runtime::platform(context);
}
