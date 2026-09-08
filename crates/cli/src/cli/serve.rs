use core::{CommandSpec, Context, FlagSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    name: "serve",
    category: "runtime",
    summary: "serve a handler or directory",
    aliases: &["start"],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--dev",
        aliases: &[],
        description: "enable development runtime mode (watch + hmr scaffolding)",
    });
}

pub fn cmd(context: &Context) {
    #[cfg(feature = "native")]
    if context.args.flags.get("--dev").copied().unwrap_or(false) {
        // Dev build-slot support (deka#725): materialize the project's build
        // slots up front so pages render without a prior `deka build`, and
        // register the watcher callback that rematerializes affected slots.
        runtime::build_watch::set_build_slot_refresh(
            crate::cli::build_slots::make_dev_refresh_callback(
                context.args.flags.clone(),
                context.args.params.clone(),
            ),
        );
        crate::cli::build_slots::ensure_dev_build_slots(
            &context.args.flags,
            &context.args.params,
            &context.handler.input,
        );
    }
    runtime::serve(context);
}
