use core::{CommandSpec, Context, FlagSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "",
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
    // `--dev` is a command-level dispatch onto the `dev` crate, not a mode
    // flag inside production serve (deka#881).
    if context.args.flags.get("--dev").copied().unwrap_or(false) {
        crate::cli::dev::run_dev(context);
        return;
    }
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
    if loose_file {
        // RFD 55: situational advisories come last of the setup output.
        stdio::note(crate::cli::user_cache::NOT_A_PROJECT_NOTE);
    }
    let dsc = if serves_built_artifact(&context) {
        None
    } else {
        match crate::dsc::find_dsc() {
            Ok(path) => path,
            Err(err) => {
                stdio::error("serve", &err);
                std::process::exit(1);
            }
        }
    };
    runtime::serve_with_dsc(&context, dsc);
}

/// A built artifact has its verified executable entry under `dist/server/`.
/// Do not resolve a compiler for this posture: the artifact-only gate removes
/// DEKA_DSC and puts a poisoned `dsc` first on PATH to prove serve cannot use
/// one.
pub(crate) fn serves_built_artifact(context: &Context) -> bool {
    let Ok(resolved) = ::run::handler::resolve_handler_path(
        &context
            .extensions()
            .get::<::run::handler::HandlerSnapshot>()
            .expect("handler snapshot populated before dispatch")
            .input,
    ) else {
        return false;
    };
    let Some(server_dir) = resolved.path.parent() else {
        return false;
    };
    server_dir.file_name().is_some_and(|name| name == "server")
        && server_dir
            .parent()
            .is_some_and(|dist| dist.join("build-manifest.json").is_file())
}

/// Resolve the handler the way `runtime::serve` will. When it is a `.ds` /
/// `.dsx` outside any project, materialize it into the user cache and return
/// a context rewritten to the compiled artifact. Project behavior is
/// untouched; `Ok(None)` means "not a loose source, use the input context".
pub(crate) fn prepare_loose_serve(context: &Context) -> Result<(Context, bool), String> {
    let resolved = ::run::handler::resolve_handler_path(
        &context
            .extensions()
            .get::<::run::handler::HandlerSnapshot>()
            .expect("handler snapshot populated before dispatch")
            .input,
    )
    .map_err(|err| format!("failed to resolve handler path: {err}"))?;
    if !crate::cli::user_cache::is_loose_source_file(&resolved.path) {
        return Ok((context.clone(), false));
    }
    let materialized =
        crate::cli::user_cache::materialize_loose(&resolved.path).map_err(|err| {
            format!(
                "failed to materialize {} into the user cache: {err}",
                resolved.path.display()
            )
        })?;
    let prepared =
        crate::cli::user_cache::rewrite_context_for_artifact(context, &materialized.artifact)?;
    Ok((prepared, true))
}
