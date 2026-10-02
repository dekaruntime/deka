//! Registry-based command dispatch for the native CLI (deka#1204).
//!
//! deka-cli-core 0.5.0 handlers are `fn(&Context)` with no return value, so
//! each handler stores its result in an [`Outcome`] slot carried in the
//! context extensions, and [`dispatch`] maps it to the process exit code in
//! exactly one place. dekaruntime/cli-core#22 (0.6.0) makes handlers return
//! a status directly; adopting it means deleting [`Outcome`] and [`finish`]
//! and returning from the handlers instead.
use crate::{Payload, Source, build, compile, dev, embedded, execute, init, source, tests};
use deka_vm::Result;
use dcore::{Args, CommandSpec, Context, FlagSpec, ParamSpec, ParseErrorKind, Registry, RegistryBuilder};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// How a failed command maps to the process exit code: 2 for usage errors
/// (GNU convention, matching the legacy CLI), 1 for runtime failures.
enum Failure {
    Usage(String),
    Runtime(String),
}

/// Handler result slot for the 0.5.0 `fn(&Context)` signature (see module doc).
#[derive(Clone, Default)]
struct Outcome(Arc<Mutex<Option<std::result::Result<(), Failure>>>>);

fn finish(ctx: &Context, result: std::result::Result<(), Failure>) {
    if let Some(slot) = ctx.extensions().get::<Outcome>() {
        *slot.0.lock().unwrap() = Some(result);
    }
}

fn runtime(result: Result<()>) -> std::result::Result<(), Failure> {
    result.map_err(Failure::Runtime)
}

fn usage(message: impl Into<String>) -> std::result::Result<(), Failure> {
    Err(Failure::Usage(message.into()))
}

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
    registry.add_param(ParamSpec {
        name: "--entry",
        description: "entry function (default: deka.json entryFunction)",
    });
    registry.add_param(ParamSpec {
        name: "--exercise",
        description: "invoke the first click handler N times without opening a window",
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
    registry.add_param(ParamSpec {
        name: "--entry",
        description: "entry function (default: deka.json entryFunction)",
    });
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
    registry.add_param(ParamSpec {
        name: "--entry",
        description: "entry function (default: deka.json entryFunction)",
    });
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
    registry.add_param(ParamSpec {
        name: "--entry",
        description: "entry function (default: deka.json entryFunction)",
    });
    registry.add_param(ParamSpec {
        name: "--outfile",
        description: "output path (default: dist/deka-app)",
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

/// Registration functions in help/ownership order (the legacy CLI pattern:
/// the ownership index re-runs these to attribute flags to their command).
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

fn wants_help(args: &Args) -> bool {
    args.flags.contains_key("--help") || args.flags.contains_key("-h") || args.flags.contains_key("help")
}

fn wants_version(args: &Args) -> bool {
    args.flags.contains_key("--version")
        || args.flags.contains_key("-v")
        || args.flags.contains_key("version")
}

fn print_version() {
    println!("deka {} (Rust VM)", env!("CARGO_PKG_VERSION"));
}

fn print_global_help(registry: &Registry) {
    for line in dcore::help::render_global_help(registry, env!("CARGO_PKG_VERSION")) {
        println!("{line}");
    }
}

/// Native command help. `dcore::help::render_command_help` is not used: its
/// curated copy describes the legacy V8 CLI (`build --bundle --minify` emits
/// JavaScript), which is wrong here — the native `build` emits a native
/// executable. This renders the registry truth: usage, summary, owned flags.
fn render_native_command_help(command: &CommandSpec, owned: Option<&dcore::help::CommandFlags>) -> Vec<String> {
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
    let index = dcore::help::build_ownership_index(&register_fns());
    match registry.command_named(name) {
        Some(command) => {
            for line in render_native_command_help(command, index.flags.get(name)) {
                println!("{line}");
            }
        }
        None => print_global_help(registry),
    }
}

fn source_arg(ctx: &Context) -> std::result::Result<Source, Failure> {
    if ctx.args.positionals.len() > 1 {
        return Err(Failure::Usage("expected one source path".into()));
    }
    if ctx.args.params.contains_key("--outfile") {
        return Err(Failure::Usage("--outfile requires build".into()));
    }
    let path = ctx
        .args
        .positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| "deka.json".into());
    source(&path, ctx.args.params.get("--entry").cloned()).map_err(Failure::Runtime)
}

fn exercise_arg(ctx: &Context) -> std::result::Result<Option<usize>, Failure> {
    ctx.args
        .params
        .get("--exercise")
        .map(|value| {
            value
                .parse()
                .map_err(|_| Failure::Usage("--exercise expects a click count".into()))
        })
        .transpose()
}

fn cmd_run(ctx: &Context) {
    let result = (|| {
        let source = source_arg(ctx)?;
        let exercise = exercise_arg(ctx)?;
        let payload = compile(&source).map_err(Failure::Runtime)?;
        runtime(execute(payload, exercise))
    })();
    finish(ctx, result);
}

fn cmd_dev(ctx: &Context) {
    let result = (|| {
        let source = source_arg(ctx)?;
        let payload = compile(&source).map_err(Failure::Runtime)?;
        if source.desktop {
            runtime(dev(source, payload))
        } else {
            runtime(execute(payload, None))
        }
    })();
    finish(ctx, result);
}

fn cmd_check(ctx: &Context) {
    let result = (|| {
        let source = source_arg(ctx)?;
        compile(&source).map_err(Failure::Runtime)?;
        println!("OK {}", source.path.display());
        Ok(())
    })();
    finish(ctx, result);
}

fn cmd_build(ctx: &Context) {
    let result = (|| {
        if ctx.args.positionals.len() > 1 {
            return usage("expected one source path");
        }
        if ctx.args.params.contains_key("--exercise") {
            return usage("--exercise requires run");
        }
        let path = ctx
            .args
            .positionals
            .first()
            .map(PathBuf::from)
            .unwrap_or_else(|| "deka.json".into());
        let source = source(&path, ctx.args.params.get("--entry").cloned())
            .map_err(Failure::Runtime)?;
        let payload = compile(&source).map_err(Failure::Runtime)?;
        let outfile = ctx
            .args
            .params
            .get("--outfile")
            .map(PathBuf::from)
            .unwrap_or_else(|| "dist/deka-app".into());
        runtime(build(payload, &outfile))
    })();
    finish(ctx, result);
}

fn cmd_test(ctx: &Context) {
    let paths: Vec<String> = if ctx.args.positionals.is_empty() {
        vec![".".into()]
    } else {
        ctx.args.positionals.clone()
    };
    finish(ctx, runtime(tests(&paths)));
}

fn cmd_init(ctx: &Context) {
    let result = (|| {
        if ctx.args.positionals.len() > 1 {
            return usage("usage: deka init [directory]");
        }
        let directory = ctx
            .args
            .positionals
            .first()
            .map(PathBuf::from)
            .unwrap_or_else(|| ".".into());
        runtime(init(&directory))
    })();
    finish(ctx, result);
}

/// Run a compiled application (its executable carries an embedded payload).
/// Kept outside the registry: an installed app has no CLI surface beyond
/// `--exercise N`, mirroring the pre-registry behavior.
fn run_embedded(payload: Payload, argv: &[String]) -> i32 {
    let exercise = match argv {
        [] => None,
        [flag, count] if flag == "--exercise" => match count.parse() {
            Ok(clicks) => Some(clicks),
            Err(_) => {
                eprintln!("deka: expected click count");
                return 2;
            }
        },
        _ => {
            eprintln!("deka: unexpected application arguments");
            return 2;
        }
    };
    match execute(payload, exercise) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("deka: {error}");
            1
        }
    }
}

/// Process entry: embedded-payload detection first (a compiled app detects
/// its trailer and runs), then registry dispatch. Returns the exit code;
/// `main` is the only caller of `std::process::exit` equivalent.
pub fn main_entry(argv: Vec<String>) -> i32 {
    if let Ok(executable) = std::env::current_exe() {
        match embedded(&executable) {
            Ok(Some(payload)) => return run_embedded(payload, &argv),
            Ok(None) => {}
            Err(error) => {
                eprintln!("deka: {error}");
                return 1;
            }
        }
    }
    dispatch(&registry(), &argv)
}

/// Dispatch one argv against the registry. argv in, exit code out, so tests
/// drive every command without spawning a process.
pub fn dispatch(registry: &Registry, argv: &[String]) -> i32 {
    let parsed = Args::collect(argv.to_vec(), registry);
    if !parsed.errors.is_empty() {
        for error in &parsed.errors {
            match &error.kind {
                ParseErrorKind::MissingParamValue { param } => {
                    eprintln!("deka: missing value for `{param}`");
                }
                ParseErrorKind::UnknownToken => {
                    eprintln!("deka: unknown argument `{}`", error.token);
                    for suggestion in &error.suggestions {
                        eprintln!("  did you mean `{suggestion}`?");
                    }
                }
            }
        }
        return 2;
    }
    let mut args = parsed.args;
    if args.commands.is_empty() {
        if wants_version(&args) {
            print_version();
            return 0;
        }
        if wants_help(&args) {
            print_global_help(registry);
            return 0;
        }
        match args.positionals.first() {
            Some(path) if path.ends_with(".ds") || path.ends_with(".dsx") => {
                args.commands.push("run".into());
            }
            Some(token) => {
                eprintln!("deka: unknown command `{token}`");
                return 2;
            }
            None => {
                print_global_help(registry);
                return 0;
            }
        }
    } else if wants_help(&args) {
        print_command_help(registry, &args.commands[0].clone());
        return 0;
    }
    let slot = Outcome::default();
    let mut context = Context::new(args);
    context.extensions_mut().insert(slot.clone());
    match registry.dispatch(&context) {
        Ok(()) => match slot.0.lock().unwrap().take() {
            None | Some(Ok(())) => 0,
            Some(Err(Failure::Usage(message))) => {
                eprintln!("deka: {message}");
                2
            }
            Some(Err(Failure::Runtime(message))) => {
                eprintln!("deka: {message}");
                1
            }
        },
        Err(error) => {
            eprintln!("deka: {error}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn run(argv: &[&str]) -> i32 {
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
        assert_eq!(run(&[]), 0);
        assert_eq!(run(&["--help"]), 0);
        assert_eq!(run(&["-h"]), 0);
        assert_eq!(run(&["help"]), 0);
        assert_eq!(run(&["--version"]), 0);
        assert_eq!(run(&["-v"]), 0);
        assert_eq!(run(&["run", "--help"]), 0);
    }

    #[test]
    fn unknown_command_and_bad_flags_are_usage_errors() {
        assert_eq!(run(&["frobnicate"]), 2);
        assert_eq!(run(&["run", "--nope"]), 2);
        assert_eq!(run(&["--outfile"]), 2);
    }

    #[test]
    fn bare_source_path_means_run() {
        let dir = tempfile::tempdir().unwrap();
        let script = write(
            dir.path(),
            "hello.ds",
            "import { echo } from \"io\";\necho(\"hi\");\n",
        );
        assert_eq!(run(&[script.to_str().unwrap()]), 0);
    }

    #[test]
    fn run_executes_script_and_maps_failures() {
        let dir = tempfile::tempdir().unwrap();
        let good = write(
            dir.path(),
            "good.ds",
            "import { echo } from \"io\";\necho(\"ok\");\n",
        );
        assert_eq!(run(&["run", good.to_str().unwrap()]), 0);
        let bad = write(dir.path(), "bad.ds", "let broken = ;\n");
        assert_eq!(run(&["run", bad.to_str().unwrap()]), 1);
        let missing = dir.path().join("missing.ds");
        assert_eq!(run(&["run", missing.to_str().unwrap()]), 1);
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
        assert_eq!(run(&["run", script, script]), 2);
        assert_eq!(run(&["run", script, "--outfile", "x"]), 2);
        assert_eq!(run(&["run", script, "--exercise", "NaN"]), 2);
    }

    #[test]
    fn check_accepts_valid_source_and_rejects_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let good = write(
            dir.path(),
            "good.ds",
            "import { echo } from \"io\";\necho(\"ok\");\n",
        );
        assert_eq!(run(&["check", good.to_str().unwrap()]), 0);
        let bad = write(dir.path(), "bad.ds", "let broken = ;\n");
        assert_eq!(run(&["check", bad.to_str().unwrap()]), 1);
    }

    #[test]
    fn init_scaffolds_once() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("app");
        assert_eq!(run(&["init", project.to_str().unwrap()]), 0);
        assert!(project.join("deka.json").exists());
        assert!(project.join("App.dsx").exists());
        assert!(project.join("app.test.ds").exists());
        assert_eq!(run(&["init", project.to_str().unwrap()]), 1);
        assert_eq!(run(&["init", "a", "b"]), 2);
    }

    #[test]
    fn test_reports_counts() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app.test.ds",
            "import { assert } from \"test\";\nfn test_addition() { assert(1 + 1 == 2); }\n",
        );
        assert_eq!(run(&["test", dir.path().to_str().unwrap()]), 0);
        write(
            dir.path(),
            "broken.test.ds",
            "import { assert } from \"test\";\nfn test_wrong() { assert(1 + 1 == 3); }\n",
        );
        assert_eq!(run(&["test", dir.path().to_str().unwrap()]), 1);
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(run(&["test", empty.path().to_str().unwrap()]), 1);
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
            0
        );
        assert!(outfile.exists());
        // The embedded path is what an installed app takes at startup: the
        // payload round-trips through the executable trailer and runs.
        let payload = embedded(&outfile).unwrap().expect("payload embedded");
        assert!(!payload.desktop);
        assert_eq!(run_embedded(payload, &[]), 0);
        let payload = embedded(&outfile).unwrap().expect("payload embedded");
        assert_eq!(run_embedded(payload, &["--bogus".to_string()]), 2);
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
