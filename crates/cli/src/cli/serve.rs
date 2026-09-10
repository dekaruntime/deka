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
    // Loose file or unbuilt loose directory (no deka.json anywhere above the
    // resolved handler): compile into the user-global cache and serve the
    // materialized artifact, leaving the user's directory untouched (deka#765).
    let (context, loose_file) = match prepare_loose_serve(context) {
        Ok(prepared) => prepared,
        Err(err) => {
            stdio::error("serve", &err);
            std::process::exit(1);
        }
    };
    #[cfg(feature = "native")]
    if !loose_file
        && context.args.flags.get("--dev").copied().unwrap_or(false)
    {
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
    if loose_file {
        // RFD 55: situational advisories come last of the setup output.
        stdio::note(crate::cli::user_cache::NOT_A_PROJECT_NOTE);
    }
    runtime::serve(&context);
}

/// Resolve the handler the way `runtime::serve` will. When it is a `.ds` /
/// `.dsx` outside any project, materialize it into the user cache and return
/// a context rewritten to the compiled artifact. Project behavior is
/// untouched; `Ok(None)` means "not a loose source, use the input context".
fn prepare_loose_serve(context: &Context) -> Result<(Context, bool), String> {
    let resolved = core::resolve_handler_path(&context.handler.input)
        .map_err(|err| format!("failed to resolve handler path: {err}"))?;
    if !crate::cli::user_cache::is_loose_source_file(&resolved.path) {
        return Ok((context.clone(), false));
    }
    let materialized = crate::cli::user_cache::materialize_loose(&resolved.path).map_err(|err| {
        format!(
            "failed to materialize {} into the user cache: {err}",
            resolved.path.display()
        )
    })?;
    let prepared =
        crate::cli::user_cache::rewrite_context_for_artifact(context, &materialized.artifact)?;
    Ok((prepared, true))
}
