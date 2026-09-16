use dcore::{CommandSpec, Context, FlagSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    owner: "compiler",
    name: "check",
    category: "project",
    summary: "validate a DekaScript file or the whole project (execs dsc)",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

const USAGE: &str = "usage: deka check <file.ds>";

/// Project source trees, the same walk `deka build` plans and emits from.
const PROJECT_SOURCE_TREES: [&str; 3] = ["app", "api", "src"];

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--as-package",
        aliases: &[],
        description: "typecheck a package working tree the way an installed consumer would resolve it",
    });
    registry.add_flag(FlagSpec {
        name: "--single-file",
        aliases: &[],
        description: "typecheck only the requested file without project imports",
    });
}

pub fn cmd(context: &Context) {
    if context.args.positionals.is_empty() {
        check_bare(context);
        return;
    }
    crate::dsc::exec_if_present();
}

/// Bare `deka check` (deka#1100): exec'ing dsc with no argument makes the
/// inner tool's usage text (`[check] usage: dsc check <file.ds>`) the first
/// thing a new user sees. Intercept instead: with a project at the cwd,
/// check every project source tree; without one, answer with deka's own
/// usage. `deka check <file>` keeps exec'ing dsc unchanged.
fn check_bare(context: &Context) {
    if context.args.flags.contains_key("--as-package")
        || context.args.flags.contains_key("--single-file")
    {
        stdio::error("check", USAGE);
        std::process::exit(2);
    }

    let root = &context.env.cwd;
    if !root.join("deka.json").is_file() {
        stdio::error("check", USAGE);
        std::process::exit(2);
    }

    let mut failures = 0usize;
    let mut checked = 0usize;
    for tree in PROJECT_SOURCE_TREES {
        let sources = match crate::dsc::collect_deka_source_files(&root.join(tree)) {
            Ok(sources) => sources,
            Err(err) => {
                stdio::error("check", &err);
                std::process::exit(1);
            }
        };
        for path in sources {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            match crate::dsc::check_path(rel, Some(root)) {
                Ok(()) => checked += 1,
                Err(diagnostic) => {
                    failures += 1;
                    stdio::error("check", &diagnostic);
                }
            }
        }
    }

    if checked == 0 {
        stdio::error(
            "check",
            "no project sources (.ds/.dsx) found in app/, api/, or src/ — check a file directly: deka check <file.ds>",
        );
        std::process::exit(1);
    }
    if failures > 0 {
        std::process::exit(1);
    }
    stdio::success(&format!("checked {checked} DekaScript source(s)"));
}
