use core::{CommandSpec, Context, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "",
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

/// `deka dev` execution path. `serve --dev` dispatches here at the command
/// level rather than stuffing `--dev` into production serve.
pub fn cmd(context: &Context) {
    run_dev(context);
}

pub(crate) fn run_dev(context: &Context) {
    // Loose file or unbuilt loose directory (no deka.json anywhere above the
    // resolved handler): compile into the user-global cache and serve the
    // materialized artifact, leaving the user's directory untouched (deka#765).
    let (context, loose_file) = match crate::cli::serve::prepare_loose_serve(context) {
        Ok(prepared) => prepared,
        Err(err) => {
            stdio::error("serve", &err);
            std::process::exit(1);
        }
    };
    #[cfg(feature = "native")]
    if !loose_file {
        // Dev build-slot support (deka#725): materialize the project's build
        // slots up front so pages render without a prior `deka build`, and
        // register the watcher callback that rematerializes affected slots.
        ::dev::build_watch::set_build_slot_refresh(
            crate::cli::build_slots::make_dev_refresh_callback(
                context.args.flags.clone(),
                context.args.params.clone(),
            ),
        );
        crate::cli::build_slots::ensure_dev_build_slots(
            &context.args.flags,
            &context.args.params,
            &context
                .extensions()
                .get::<::run::handler::HandlerSnapshot>()
                .expect("handler snapshot populated before dispatch")
                .input,
        );
    }
    if loose_file {
        // RFD 55: situational advisories come last of the setup output.
        stdio::note(crate::cli::user_cache::NOT_A_PROJECT_NOTE);
    }
    let dsc = if crate::cli::serve::serves_built_artifact(&context) {
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
    ::dev::serve_with_dsc(&context, dsc);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_name_is_dev() {
        assert_eq!(COMMAND.name, "dev");
        assert_eq!(COMMAND.category, "runtime");
    }
}
