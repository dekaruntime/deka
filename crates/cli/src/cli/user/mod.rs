use core::{CommandSpec, Context, Registry, SubcommandSpec};

const COMMAND: CommandSpec = CommandSpec {
    name: "user",
    category: "users",
    summary: "manage users",
    aliases: &[],
    subcommands: &[ADD_SUBCOMMAND],
    handler: cmd,
};

const ADD_SUBCOMMAND: SubcommandSpec = SubcommandSpec {
    name: "add",
    summary: "add a user",
    aliases: &["create"],
    handler: cmd_add,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

pub fn cmd(context: &Context) {
    for (key, value) in context.args.params.iter() {
        println!("user command");
        println!("Key: {}, Value: {}", key, value);
    }
}

fn cmd_add(context: &Context) {
    for (key, value) in context.args.params.iter() {
        println!("user add");
        println!("Key: {}, Value: {}", key, value);
    }
}
