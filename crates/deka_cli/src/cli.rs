//! Registry-based command dispatch for the native CLI (deka#1204).
//!
//! Built on the deka-cli-core 0.6.0 contract (dekaruntime/cli-core#22):
//! handlers return `Result<ExitStatus, CommandError>`, and
//! `Registry::run_argv` is the single place that prints errors and maps
//! results to the process exit code. [`dispatch`] adds only the pre-registry
//! conveniences (bare `.ds`/`.dsx` means `run`, help/version, global help on
//! an empty invocation) and delegates everything else.
//!
//! Help renders locally: `dcore`'s renderers are pinned to deka-cli-core
//! 0.5.0 for the legacy V8 CLI (and its curated copy describes that CLI),
//! so the native CLI renders its own from this registry.
use crate::{Payload, Source, build, compile, dev, embedded, execute, init, source, tests};
use deka_cli_core::registry::{
    Args, CommandError, CommandSpec, Context, ExitStatus, FlagSpec, HandlerResult, ParamKind,
    ParamSpec, Registry, RegistryBuilder,
};
use deka_vm::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

fn register_global(registry: &mut Registry) {
    registry.add_flag(FlagSpec {
        name: "--help",
        aliases: &["-h", "help"],
        description: "show help",
    });
    registry.add_flag(FlagSpec {
        name: "--version",
        aliases: &["-v", "version"],
        description: "show version",
    });
}

fn entry_param(registry: &mut Registry) {
    registry.add_param(ParamSpec {
        name: "--entry",
        description: "entry function (default: deka.json entryFunction)",
        kind: ParamKind::Value,
    });
}

fn register_run(registry: &mut Registry) {
    registry.add_command(CommandSpec {
        name: "run",
        owner: "deka_cli",
        category: "Core",
        summary: "run a .ds/.dsx script or desktop app",
        aliases: &["start"],
        subcommands: &[],
        handler: cmd_run,
    });
    entry_param(registry);
    registry.add_param(ParamSpec {
        name: "--exercise",
        description: "invoke the first click handler N times without opening a window",
        kind: ParamKind::Value,
    });
}

fn register_dev(registry: &mut Registry) {
    registry.add_command(CommandSpec {
        name: "dev",
        owner: "deka_cli",
        category: "Core",
        summary: "run a desktop app, restarting state on source edits",
        aliases: &[],
        subcommands: &[],
        handler: cmd_dev,
    });
    entry_param(registry);
}

fn register_check(registry: &mut Registry) {
    registry.add_command(CommandSpec {
        name: "check",
        owner: "deka_cli",
        category: "Core",
        summary: "compile a script or app without running it",
        aliases: &[],
        subcommands: &[],
        handler: cmd_check,
    });
    entry_param(registry);
}

fn register_build(registry: &mut Registry) {
    registry.add_command(CommandSpec {
        name: "build",
        owner: "deka_cli",
        category: "Core",
        summary: "compile to a single native executable",
        aliases: &["compile"],
        subcommands: &[],
        handler: cmd_build,
    });
    entry_param(registry);
    registry.add_param(ParamSpec {
        name: "--outfile",
        description: "output path (default: dist/deka-app)",
        kind: ParamKind::Value,
    });
}

fn register_test(registry: &mut Registry) {
    registry.add_command(CommandSpec {
        name: "test",
        owner: "deka_cli",
        category: "Core",
        summary: "run *.test.ds/*.spec.ds test functions",
        aliases: &[],
        subcommands: &[],
        handler: cmd_test,
    });
}

fn register_init(registry: &mut Registry) {
    registry.add_command(CommandSpec {
        name: "init",
        owner: "deka_cli",
        category: "Core",
        summary: "scaffold a desktop project in a directory",
        aliases: &[],
        subcommands: &[],
        handler: cmd_init,
    });
}

/// Registration functions in help/ownership order (the ownership index
/// re-runs these to attribute flags to their command).
pub fn register_fns() -> Vec<fn(&mut Registry)> {
    vec![
        register_global,
        register_run,
        register_dev,
        register_check,
        register_build,
        register_test,
        register_init,
    ]
}

pub fn registry() -> Registry {
    let mut builder = RegistryBuilder::new();
    for register in register_fns() {
        builder = builder.with(register);
    }
    builder.build().expect("deka_cli registry")
}

struct CommandFlags {
    flags: Vec<FlagSpec>,
    params: Vec<ParamSpec>,
}

/// Which flags/params each command owns, derived by re-running every
/// registration function against a scratch registry (same technique as the
/// legacy CLI's ownership index) so `deka <command> --help` shows a
/// command's own flags rather than every same-named one.
fn ownership_index() -> HashMap<&'static str, CommandFlags> {
    let mut index = HashMap::new();
    for register in register_fns() {
        let mut scratch = Registry::new();
        register(&mut scratch);
        let owned = CommandFlags {
            flags: scratch.flags().to_vec(),
            params: scratch.params().to_vec(),
        };
        for command in scratch.commands() {
            index.insert(
                command.name,
                CommandFlags {
                    flags: owned.flags.clone(),
                    params: owned.params.clone(),
                },
            );
        }
    }
    index
}

fn print_version() {
    println!("deka {} (Rust VM)", env!("CARGO_PKG_VERSION"));
}

fn render_global_help(registry: &Registry) -> Vec<String> {
    let mut lines = vec![
        "Usage: deka [options] [command]".to_string(),
        format!(
            "deka v{} — native DekaScript runtime",
            env!("CARGO_PKG_VERSION")
        ),
        String::new(),
    ];
    let mut grouped: std::collections::BTreeMap<&str, Vec<&CommandSpec>> =
        std::collections::BTreeMap::new();
    for command in registry.commands() {
        grouped.entry(command.category).or_default().push(command);
    }
    for (category, commands) in grouped {
        lines.push(category.to_string());
        for command in commands {
            lines.push(format!("  {}\t\t{}", command.name, command.summary));
        }
        lines.push(String::new());
    }
    lines.push("flags".to_string());
    for flag in registry.flags() {
        lines.push(format!("  {}\t\t{}", flag.name, flag.description));
    }
    lines.push(String::new());
    lines.push("Run `deka <command> --help` for a command's own flags.".to_string());
    lines
}

fn print_global_help(registry: &Registry) {
    for line in render_global_help(registry) {
        println!("{line}");
    }
}

fn render_command_help(command: &CommandSpec, owned: Option<&CommandFlags>) -> Vec<String> {
    let arguments = match command.name {
        "test" => "[files or directories]",
        "init" => "[directory]",
        _ => "[source.ds | deka.json]",
    };
    let mut lines = vec![
        format!("Usage: deka {} {} [options]", command.name, arguments),
        String::new(),
        command.summary.to_string(),
        String::new(),
    ];
    if let Some(owned) = owned
        && (!owned.flags.is_empty() || !owned.params.is_empty())
    {
        lines.push("Flags:".to_string());
        for flag in &owned.flags {
            lines.push(format!("  {}\t\t{}", flag.name, flag.description));
        }
        for param in &owned.params {
            lines.push(format!("  {} <value>\t{}", param.name, param.description));
        }
        lines.push(String::new());
    }
    lines.push("Run `deka --help` to see every command.".to_string());
    lines
}

fn print_command_help(registry: &Registry, name: &str) {
    let index = ownership_index();
    match registry.command_named(name) {
        Some(command) => {
            for line in render_command_help(command, index.get(name)) {
                println!("{line}");
            }
        }
        None => print_global_help(registry),
    }
}

fn runtime(result: Result<()>) -> HandlerResult {
    // Execution failures carry the legacy runtime's "Run failed:" marker so
    // they read as run-time errors, distinct from compile errors (the corpus
    // gate stages on it).
    result
        .map(|()| ExitStatus::SUCCESS)
        .map_err(|error| CommandError::runtime(format!("Run failed: {error}")))
}

fn source_arg(ctx: &Context) -> std::result::Result<Source, CommandError> {
    if ctx.args.positionals.len() > 1 {
        return Err(CommandError::usage("expected one source path"));
    }
    if ctx.args.params.contains_key("--outfile") {
        return Err(CommandError::usage("--outfile requires build"));
    }
    let path = ctx
        .args
        .positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| "deka.json".into());
    source(&path, ctx.param::<String>("--entry")?).map_err(CommandError::Runtime)
}

fn cmd_run(ctx: &Context) -> HandlerResult {
    let source = source_arg(ctx)?;
    let exercise = ctx.param::<usize>("--exercise")?;
    let payload = compile(&source).map_err(CommandError::Runtime)?;
    runtime(execute(payload, exercise))
}

fn cmd_dev(ctx: &Context) -> HandlerResult {
    let source = source_arg(ctx)?;
    let payload = compile(&source).map_err(CommandError::Runtime)?;
    if source.desktop {
        runtime(dev(source, payload))
    } else {
        runtime(execute(payload, None))
    }
}

fn cmd_check(ctx: &Context) -> HandlerResult {
    let source = source_arg(ctx)?;
    compile(&source).map_err(CommandError::Runtime)?;
    ctx.out()
        .print(format_args!("OK {}\n", source.path.display()));
    Ok(ExitStatus::SUCCESS)
}

fn cmd_build(ctx: &Context) -> HandlerResult {
    if ctx.args.positionals.len() > 1 {
        return Err(CommandError::usage("expected one source path"));
    }
    if ctx.args.params.contains_key("--exercise") {
        return Err(CommandError::usage("--exercise requires run"));
    }
    let path = ctx
        .args
        .positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| "deka.json".into());
    let source = source(&path, ctx.param::<String>("--entry")?).map_err(CommandError::Runtime)?;
    let payload = compile(&source).map_err(CommandError::Runtime)?;
    let outfile = ctx
        .param::<PathBuf>("--outfile")?
        .unwrap_or_else(|| "dist/deka-app".into());
    runtime(build(payload, &outfile, ctx.out()))
}

fn cmd_test(ctx: &Context) -> HandlerResult {
    let paths: Vec<String> = if ctx.args.positionals.is_empty() {
        vec![".".into()]
    } else {
        ctx.args.positionals.clone()
    };
    runtime(tests(&paths, ctx.out(), ctx.err()))
}

fn cmd_init(ctx: &Context) -> HandlerResult {
    if ctx.args.positionals.len() > 1 {
        return Err(CommandError::usage("usage: deka init [directory]"));
    }
    let directory = ctx
        .args
        .positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    runtime(init(&directory))
}

/// Run a compiled application (its executable carries an embedded payload).
/// Kept outside the registry: an installed app has no CLI surface beyond
/// `--exercise N`, mirroring the pre-registry behavior.
fn run_embedded(payload: Payload, argv: &[String]) -> ExitCode {
    let exercise = match argv {
        [] => None,
        [flag, count] if flag == "--exercise" => match count.parse() {
            Ok(clicks) => Some(clicks),
            Err(_) => {
                eprintln!("deka: expected click count");
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("deka: unexpected application arguments");
            return ExitCode::from(2);
        }
    };
    match execute(payload, exercise) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("deka: {error}");
            ExitCode::from(1)
        }
    }
}

/// Process entry: embedded-payload detection first (a compiled app detects
/// its trailer and runs), then registry dispatch. `main` is the single
/// exit point.
pub fn main_entry(argv: Vec<String>) -> ExitCode {
    if let Ok(executable) = std::env::current_exe() {
        match embedded(&executable) {
            Ok(Some(payload)) => return run_embedded(payload, &argv),
            Ok(None) => {}
            Err(error) => {
                eprintln!("deka: {error}");
                return ExitCode::from(1);
            }
        }
    }
    dispatch(&registry(), &argv)
}

/// Dispatch one argv against the registry. Handles the pre-registry
/// conveniences (empty invocation, help/version, bare `.ds`/`.dsx` path)
/// and delegates parsing, error printing and exit-code mapping to
/// `Registry::run_argv`. argv in, exit code out, so tests drive every
/// command without spawning a process.
pub fn dispatch(registry: &Registry, argv: &[String]) -> ExitCode {
    if argv.is_empty() {
        print_global_help(registry);
        return ExitCode::SUCCESS;
    }
    let parsed = Args::collect(argv.to_vec(), registry);
    if parsed.errors.is_empty() {
        let args = &parsed.args;
        if args.commands.is_empty() {
            if args.flags.contains_key("--version") {
                print_version();
                return ExitCode::SUCCESS;
            }
            if args.flags.contains_key("--help") {
                print_global_help(registry);
                return ExitCode::SUCCESS;
            }
            match args.positionals.first() {
                Some(path) if path.ends_with(".ds") || path.ends_with(".dsx") => {
                    let mut rewritten = vec!["run".to_string()];
                    rewritten.extend_from_slice(argv);
                    return registry.run_argv(&rewritten);
                }
                _ => {
                    print_global_help(registry);
                    return ExitCode::SUCCESS;
                }
            }
        }
        if args.flags.contains_key("--help") {
            print_command_help(registry, &args.commands[0]);
            return ExitCode::SUCCESS;
        }
    }
    registry.run_argv(argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn run(argv: &[&str]) -> ExitCode {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        dispatch(&registry(), &argv)
    }

    fn write(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn help_version_and_bare_invocation_succeed() {
        assert_eq!(run(&[]), ExitCode::SUCCESS);
        assert_eq!(run(&["--help"]), ExitCode::SUCCESS);
        assert_eq!(run(&["-h"]), ExitCode::SUCCESS);
        assert_eq!(run(&["help"]), ExitCode::SUCCESS);
        assert_eq!(run(&["--version"]), ExitCode::SUCCESS);
        assert_eq!(run(&["-v"]), ExitCode::SUCCESS);
        assert_eq!(run(&["run", "--help"]), ExitCode::SUCCESS);
    }

    #[test]
    fn unknown_command_and_bad_flags_are_usage_errors() {
        assert_eq!(run(&["frobnicate"]), ExitCode::from(2));
        assert_eq!(run(&["run", "--nope"]), ExitCode::from(2));
        assert_eq!(run(&["--outfile"]), ExitCode::from(2));
        let registry = registry();
        let (code, _out, err) = registry.run_captured(&["frobnicate".to_string()]);
        assert_eq!(code, ExitCode::from(2));
        assert!(err.string().contains("unknown argument `frobnicate`"));
    }

    #[test]
    fn bare_source_path_means_run() {
        let dir = tempfile::tempdir().unwrap();
        let script = write(
            dir.path(),
            "hello.ds",
            "import { echo } from \"io\";\necho(\"hi\");\n",
        );
        assert_eq!(run(&[script.to_str().unwrap()]), ExitCode::SUCCESS);
    }

    #[test]
    fn run_executes_script_and_maps_failures() {
        let dir = tempfile::tempdir().unwrap();
        let good = write(
            dir.path(),
            "good.ds",
            "import { echo } from \"io\";\necho(\"ok\");\n",
        );
        assert_eq!(run(&["run", good.to_str().unwrap()]), ExitCode::SUCCESS);
        let bad = write(dir.path(), "bad.ds", "let broken = ;\n");
        assert_eq!(run(&["run", bad.to_str().unwrap()]), ExitCode::from(1));
        let missing = dir.path().join("missing.ds");
        assert_eq!(run(&["run", missing.to_str().unwrap()]), ExitCode::from(1));
    }

    #[test]
    fn run_rejects_usage_mistakes() {
        let dir = tempfile::tempdir().unwrap();
        let script = write(
            dir.path(),
            "hello.ds",
            "import { echo } from \"io\";\necho(\"hi\");\n",
        );
        let script = script.to_str().unwrap();
        assert_eq!(run(&["run", script, script]), ExitCode::from(2));
        assert_eq!(run(&["run", script, "--outfile", "x"]), ExitCode::from(2));
        let (code, _out, err) = registry().run_captured(&[
            "run".into(),
            script.into(),
            "--exercise".into(),
            "NaN".into(),
        ]);
        assert_eq!(code, ExitCode::from(2));
        assert!(
            err.string().starts_with("invalid value for `--exercise`: "),
            "{}",
            err.string()
        );
    }

    #[test]
    fn check_accepts_valid_source_and_rejects_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let good = write(
            dir.path(),
            "good.ds",
            "import { echo } from \"io\";\necho(\"ok\");\n",
        );
        assert_eq!(run(&["check", good.to_str().unwrap()]), ExitCode::SUCCESS);
        let (code, out, _err) =
            registry().run_captured(&["check".into(), good.to_str().unwrap().into()]);
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(out.string().starts_with("OK "), "{}", out.string());
        let bad = write(dir.path(), "bad.ds", "let broken = ;\n");
        assert_eq!(run(&["check", bad.to_str().unwrap()]), ExitCode::from(1));
    }

    #[test]
    fn init_scaffolds_once() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("app");
        assert_eq!(run(&["init", project.to_str().unwrap()]), ExitCode::SUCCESS);
        assert!(project.join("deka.json").exists());
        assert!(project.join("App.dsx").exists());
        assert!(project.join("app.test.ds").exists());
        assert_eq!(run(&["init", project.to_str().unwrap()]), ExitCode::from(1));
        assert_eq!(run(&["init", "a", "b"]), ExitCode::from(2));
    }

    #[test]
    fn test_reports_counts() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app.test.ds",
            "import { assert } from \"test\";\nfn test_addition() { assert(1 + 1 == 2); }\n",
        );
        let path = dir.path().to_str().unwrap().to_string();
        assert_eq!(run(&["test", &path]), ExitCode::SUCCESS);
        let (code, out, _err) = registry().run_captured(&["test".into(), path.clone()]);
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(
            out.string().contains("1 passed, 0 failed"),
            "{}",
            out.string()
        );
        write(
            dir.path(),
            "broken.test.ds",
            "import { assert } from \"test\";\nfn test_wrong() { assert(1 + 1 == 3); }\n",
        );
        assert_eq!(run(&["test", &path]), ExitCode::from(1));
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            run(&["test", empty.path().to_str().unwrap()]),
            ExitCode::from(1)
        );
    }

    #[test]
    fn build_produces_an_executable_with_an_embedded_payload() {
        let dir = tempfile::tempdir().unwrap();
        let script = write(
            dir.path(),
            "hello.ds",
            "import { echo } from \"io\";\necho(\"built\");\n",
        );
        let outfile = dir.path().join("hello-app");
        assert_eq!(
            run(&[
                "build",
                script.to_str().unwrap(),
                "--outfile",
                outfile.to_str().unwrap()
            ]),
            ExitCode::SUCCESS
        );
        assert!(outfile.exists());
        // The embedded path is what an installed app takes at startup: the
        // payload round-trips through the executable trailer and runs.
        let payload = embedded(&outfile).unwrap().expect("payload embedded");
        assert!(!payload.desktop);
        assert_eq!(run_embedded(payload, &[]), ExitCode::SUCCESS);
        let payload = embedded(&outfile).unwrap().expect("payload embedded");
        assert_eq!(
            run_embedded(payload, &["--bogus".to_string()]),
            ExitCode::from(2)
        );
    }

    #[test]
    fn registry_has_the_full_command_surface() {
        let registry = registry();
        for name in ["run", "dev", "check", "build", "test", "init"] {
            assert!(registry.command_named(name).is_some(), "missing {name}");
        }
        let run = registry.command_named("run").unwrap();
        assert!(run.aliases.contains(&"start"));
        let build = registry.command_named("build").unwrap();
        assert!(build.aliases.contains(&"compile"));
    }
}
