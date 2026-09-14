use core::{FlagSpec, ParamSpec, ParseError, ParseErrorKind, Registry};
use stdio::{ascii, error as stdio_error, raw};

pub fn register_global_flags(registry: &mut Registry) {
    registry.add_flag(FlagSpec {
        name: "--help",
        aliases: &["-H", "help"],
        description: "show help",
    });
    registry.add_flag(FlagSpec {
        name: "--version",
        aliases: &["-V", "version"],
        description: "show version",
    });
    registry.add_flag(FlagSpec {
        name: "--verbose",
        aliases: &[],
        description: "show detailed metadata where supported",
    });
    registry.add_flag(FlagSpec {
        name: "--debug",
        aliases: &["-d", "debug"],
        description: "enable debug logging",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-read",
        aliases: &[],
        description: "allow filesystem reads (`--allow-read=./src,./data`)",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-write",
        aliases: &[],
        description: "allow filesystem writes",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-net",
        aliases: &[],
        description: "compatibility flag; serve/run/platform read net policy from deka.json",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-env",
        aliases: &[],
        description: "compatibility flag; serve/run/platform read env policy from deka.json",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-run",
        aliases: &[],
        description: "allow subprocess execution",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-db",
        aliases: &[],
        description: "allow database operations",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-dynamic",
        aliases: &[],
        description: "allow dynamic code execution",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-wasm",
        aliases: &[],
        description: "allow wasm module load and execution",
    });
    registry.add_flag(FlagSpec {
        name: "--allow-all",
        aliases: &[],
        description: "allow all security capabilities",
    });
    registry.add_flag(FlagSpec {
        name: "--deny-read",
        aliases: &[],
        description: "deny filesystem reads (`--deny-read=/etc`)",
    });
    registry.add_flag(FlagSpec {
        name: "--deny-write",
        aliases: &[],
        description: "deny filesystem writes",
    });
    registry.add_flag(FlagSpec {
        name: "--deny-net",
        aliases: &[],
        description: "compatibility flag; serve/run/platform read net policy from deka.json",
    });
    registry.add_flag(FlagSpec {
        name: "--deny-env",
        aliases: &[],
        description: "compatibility flag; serve/run/platform read env policy from deka.json",
    });
    registry.add_flag(FlagSpec {
        name: "--deny-run",
        aliases: &[],
        description: "deny subprocess execution",
    });
    registry.add_flag(FlagSpec {
        name: "--deny-db",
        aliases: &[],
        description: "deny database operations",
    });
    registry.add_flag(FlagSpec {
        name: "--deny-dynamic",
        aliases: &[],
        description: "deny dynamic code execution",
    });
    registry.add_flag(FlagSpec {
        name: "--deny-wasm",
        aliases: &[],
        description: "deny wasm module load and execution",
    });
    registry.add_flag(FlagSpec {
        name: "--no-prompt",
        aliases: &[],
        description: "disable interactive security prompts",
    });
}

pub fn register_global_params(registry: &mut Registry) {
    registry.add_param(ParamSpec {
        name: "--port",
        description: "server port",
    });
    registry.add_param(ParamSpec {
        name: "--mode",
        description: "runtime mode",
    });
    registry.add_param(ParamSpec {
        name: "--folder",
        description: "target folder",
    });
    registry.add_param(ParamSpec {
        name: "--outdir",
        description: "build output directory",
    });
    registry.add_param(ParamSpec {
        name: "-o",
        description: "build output directory",
    });
    registry.add_param(ParamSpec {
        name: "--username",
        description: "linkhash username (recommended format: @username)",
    });
    registry.add_param(ParamSpec {
        name: "--token",
        description: "linkhash auth token",
    });
    registry.add_param(ParamSpec {
        name: "--email",
        description: "account email",
    });
    registry.add_param(ParamSpec {
        name: "--password",
        description: "account password",
    });
    registry.add_param(ParamSpec {
        name: "--registry-url",
        description: "linkhash registry base URL",
    });
    registry.add_param(ParamSpec {
        name: "--rust",
        description: "emit a built-in Rust seam contract target",
    });
}

// provide helpful info if no args are provided
pub fn help(registry: &Registry) {
    raw(&ascii("deka"));
    raw("");
    for line in core::help::render_global_help(registry, env!("CARGO_PKG_VERSION")) {
        raw(&line);
    }
}

/// `deka <command> --help`: the command's own usage, description, flags,
/// and (for the commands a new user reaches for first) worked examples —
/// no longer a byte-identical copy of `deka --help` (deka#977). `owned` is
/// that command's own flags/params, resolved by re-running its own
/// registration function (see `crate::command_flag_index`), never by
/// name-searching the shared registry — two different commands can
/// register a flag with the same name (deka#996 review).
pub fn command_help(command: &core::CommandSpec, owned: Option<&core::help::CommandFlags>) {
    for line in core::help::render_command_help(command, owned) {
        raw(&line);
    }
}

pub fn version(verbose: bool) {
    let version = env!("CARGO_PKG_VERSION");
    raw(&format!("deka [version {}]", version));
    if verbose {
        let git_sha = option_env!("DEKA_GIT_SHA").unwrap_or("unknown");
        let build_unix = option_env!("DEKA_BUILD_UNIX").unwrap_or("unknown");
        let target = option_env!("DEKA_TARGET").unwrap_or("unknown");
        let runtime_abi = option_env!("DEKA_RUNTIME_ABI").unwrap_or("unknown");
        raw(&format!("git_sha: {}", git_sha));
        raw(&format!("build_unix: {}", build_unix));
        raw(&format!("target: {}", target));
        raw(&format!("runtime_abi: {}", runtime_abi));
        let react = option_env!("DEKA_REACT_VERSION").unwrap_or("unknown");
        raw(&format!("react: {}", react));
    }
    raw("");
}

pub fn error(msg: Option<&str>) {
    stdio_error(
        "cli",
        msg.unwrap_or("instructions unclear. try '--help' for guidance"),
    );
}

/// Dispatches argv. Returns the process exit code: 0 on success, 2 on any
/// usage error (unknown argument/flag, missing param value, unknown command),
/// following the GNU exit-code convention. Command handlers keep owning
/// runtime failures (they exit 1 themselves).
pub fn execute(registry: &Registry) -> i32 {
    #[cfg(feature = "native")]
    {
        if runtime::has_embedded_vfs() {
            let args = std::env::args().skip(1).collect::<Vec<_>>();
            if let Err(err) = runtime::run_embedded_vfs(args) {
                error(Some(err.as_str()));
                return 1;
            }
            return 0;
        }
    }

    let ownership_index = crate::command_flag_index();
    let parsed = core::parse_env(registry);
    if !parsed.errors.is_empty() {
        let message = format_parse_errors(registry, &ownership_index, &parsed.errors);
        error(Some(message.as_str()));
        return 2;
    }

    let args = &parsed.args;
    // Single commands never reached --help before: `init` writes a project
    // and every other handler ran unconditionally. Intercept help first so
    // `deka build --help` prints usage and exits 0 even outside a project.
    // Compiler commands delegate their help to dsc for command-specific
    // output, so they keep passing through.
    if single_command_wants_help(args) {
        match registry.command_named(&args.commands[0]) {
            Some(command) => {
                let index = crate::command_flag_index();
                command_help(command, index.get(command.name));
            }
            None => help(registry),
        }
        return 0;
    }

    if args.commands.is_empty() {
        if args.flags.is_empty() {
            help(registry);
            return 0;
        }
        if args.flags.contains_key("--version")
            || args.flags.contains_key("-V")
            || args.flags.contains_key("version")
        {
            let verbose = args.flags.contains_key("--verbose");
            version(verbose);
            return 0;
        }
    }

    let context = match crate::context::from_env(registry) {
        Ok(context) => context,
        Err(crate::context::ContextError::Parse(errors)) => {
            let message = format_parse_errors(registry, &ownership_index, &errors);
            error(Some(message.as_str()));
            return 2;
        }
        Err(crate::context::ContextError::HandlerResolve(message)) => {
            error(Some(message.as_str()));
            return 2;
        }
    };
    let cmd = &context.args;
    // check if there are any command-line arguments provided
    if cmd.commands.is_empty() {
        // returning help if no commands or flags are provided, else check for flags that return content to user
        if cmd.flags.is_empty() {
            help(registry);
        } else {
            if cmd.flags.contains_key("--help")
                || cmd.flags.contains_key("-H")
                || cmd.flags.contains_key("help")
            {
                help(registry);
            }
            if cmd.flags.contains_key("--version")
                || cmd.flags.contains_key("-V")
                || cmd.flags.contains_key("version")
            {
                let verbose = cmd.flags.contains_key("--verbose");
                version(verbose);
            }
        }
        return 0;
    } else {
        if cmd.commands.len() > 2 {
            error(None);
            return 2;
        }

        let cmd_name = &cmd.commands[0];
        let Some(command) = registry.command_named(cmd_name) else {
            error(None);
            return 2;
        };

        if cmd.commands.len() == 1 {
            (command.handler)(&context);
            return 0;
        }

        let sub_name = &cmd.commands[1];
        let Some(subcommand) = registry.subcommand_named(command, sub_name) else {
            error(None);
            return 2;
        };

        (subcommand.handler)(&context);
        return 0;
    }
}

pub(crate) fn single_command_wants_help(args: &core::Args) -> bool {
    if args.commands.len() != 1 {
        return false;
    }
    if matches!(
        args.commands[0].as_str(),
        "check" | "fmt" | "transpile" | "lsp"
    ) {
        return false;
    }
    args.flags.contains_key("--help")
        || args.flags.contains_key("-H")
        || args.flags.contains_key("help")
}

/// Render parse-error messages from real CLI parse outcomes.
pub fn format_parse_errors(
    registry: &Registry,
    ownership_index: &std::collections::HashMap<&'static str, core::help::CommandFlags>,
    errors: &[ParseError],
) -> String {
    const SUGGESTION_LIMIT: usize = 3;
    let mut output = String::new();
    for error in errors {
        match &error.kind {
            ParseErrorKind::UnknownToken => {
                output.push_str(&format!("unknown argument '{}'", error.token));
                if !error.suggestions.is_empty() {
                    output.push_str(". did you mean ");
                    let suggestions = core::help::expand_suggestions(
                        registry,
                        ownership_index,
                        &error.suggestions,
                    );
                    output.push_str(&core::help::format_suggestions(
                        &suggestions,
                        SUGGESTION_LIMIT,
                    ));
                    output.push('?');
                }
                output.push('\n');
            }
            ParseErrorKind::MissingParamValue { param } => {
                output.push_str(&format!("missing value for '{}'\n", param));
            }
        }
    }
    output
}
