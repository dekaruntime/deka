//! Help content for the `deka` CLI.
//!
//! Rendering here is pure text formatting over `Registry` data — no I/O.
//! `cli::mod` is the only place that prints; keeping the formatting in
//! `core` keeps `crates/cli/src` inside its composition-only line budget
//! (see `scripts/check-cli-surface.sh`).
//!
//! Two kinds of content live here:
//! - a hand-written [`CommandHelp`] for the commands a brand-new user
//!   touches first (deka#977) — real usage lines and copy-pasteable
//!   examples, not generated from the registry.
//! - a generic fallback for every other registered command, built from
//!   whatever the registry actually has (summary, subcommands, and any
//!   flags/params this module knows that command owns) so help never
//!   claims a flag or subcommand that isn't really there.

use crate::{CommandSpec, FlagSpec, ParamSpec, Registry};

/// Curated help for a single command: a real usage line, a plain-English
/// description, and copy-pasteable examples. Flags are resolved from the
/// live registry by name (see [`command_flag_names`]) so the description
/// text shown never drifts from what the flag actually says.
pub struct CommandHelp {
    pub usage: &'static str,
    pub about: &'static str,
    pub examples: &'static [(&'static str, &'static str)],
}

/// Curated help for the commands a new user reaches for first. Anything
/// not listed here falls back to the generic path in [`render_command_help`].
fn curated_help(name: &str) -> Option<CommandHelp> {
    match name {
        "init" => Some(CommandHelp {
            usage: "deka init [directory]",
            about: "Scaffold a new deka project: deka.json, deka.lock, .gitignore, \
                    index.html, and starter source. Run it with no arguments to \
                    initialize the current directory.",
            examples: &[
                ("deka init", "scaffold a project in the current directory"),
                ("deka init my-shop", "scaffold a new project in ./my-shop"),
            ],
        }),
        "serve" => Some(CommandHelp {
            usage: "deka serve [directory|file] [--dev] [--port <port>]",
            about: "Serve a deka project (or a single loose file) as an HTTP server. \
                    With no arguments it serves the current directory. Add --dev for \
                    file-watching and hot reload — that's the same server `deka dev` \
                    runs.",
            examples: &[
                ("deka serve", "serve the current project on port 8530"),
                (
                    "deka serve --dev",
                    "serve with file-watching and hot reload",
                ),
                ("deka serve --port 3000", "serve on a specific port"),
            ],
        }),
        "dev" => Some(CommandHelp {
            usage: "deka dev [directory|file]",
            about: "Serve your project with hot module reload while you edit — this \
                    is `deka serve --dev` under a shorter name. Use it as your everyday \
                    local development loop.",
            examples: &[("deka dev", "serve the current project with hot reload")],
        }),
        "run" => Some(CommandHelp {
            usage: "deka run <file> [--watch] [-- <args>]",
            about: "Run a single .ds/.dsx/.js file, or a script named in deka.json. \
                    The file does not need to belong to a project — a loose file \
                    compiles into a user-global cache and runs from there, without \
                    writing anything into your directory.",
            examples: &[
                ("deka run app.ds", "run a single file"),
                (
                    "deka run --watch server.ds",
                    "run and restart on file changes",
                ),
            ],
        }),
        "build" => Some(CommandHelp {
            usage: "deka build [file] [--bundle] [--minify] [--out <path>]",
            about: "Compile your project — or a single .ds file — into JavaScript, \
                    ready to deploy. Use --bundle to emit one resolved module graph \
                    instead of a tree of files.",
            examples: &[
                ("deka build", "build the current project"),
                (
                    "deka build --bundle --minify",
                    "build one minified bundle for production",
                ),
            ],
        }),
        "install" | "add" | "i" => Some(CommandHelp {
            usage: "deka install | deka add <package> [<package>...]",
            about: "`deka install` reads deka.json and populates ds_modules/ with \
                    every declared dependency. `deka add` (alias: `i`) fetches one or \
                    more packages from the registry and adds them to deka.json.",
            examples: &[
                (
                    "deka install",
                    "install every dependency declared in deka.json",
                ),
                ("deka add @deka/io", "add a single package"),
                (
                    "deka add @deka/io @deka/json",
                    "add more than one package at once",
                ),
            ],
        }),
        _ => None,
    }
}

/// Flag/param names this module knows belong to a given command, used to
/// build the "Flags" section of the generic fallback. Only names go here —
/// the description text is always read live off the registry, so it can
/// never drift out of sync with the real flag.
fn command_flag_names(name: &str) -> &'static [&'static str] {
    match name {
        "serve" => &["--dev"],
        "dev" => &["--dev"],
        "run" => &["--watch", "-W"],
        "build" => &["--bundle", "--minify", "--out"],
        "install" | "add" | "i" | "update" => &[
            "--quiet",
            "-q",
            "--yes",
            "-y",
            "--prompt",
            "-p",
            "--rehash",
            "--locked",
            "--payload",
            "--spec",
            "--concurrency",
            "--registry",
            "--token",
        ],
        "deploy" => &["--gild-socket", "--gild-bearer", "--run-id"],
        "task" => &["--list", "--json"],
        "compile" => &["--outfile", "--desktop"],
        "lsp" => &["--stdio"],
        "transpile" => &["--preserve", "--bundle", "--treeshake", "--client", "--out"],
        "check" => &["--as-package", "--single-file"],
        "test" => &["--test-name-pattern", "-t"],
        "introspect" => &[
            "--archive",
            "--json",
            "--runtime",
            "-r",
            "--sort",
            "-s",
            "--limit",
            "-l",
        ],
        "fmt" => &["--lang", "--check", "--stdin"],
        "release" => &[
            "--name",
            "--repo",
            "--version",
            "--pkg-version",
            "--bump",
            "--token",
            "--registry-url",
            "--description",
            "--no-push",
            "--yes",
        ],
        "publish" => &[
            "--name",
            "--version",
            "--pkg-version",
            "--repo",
            "--git-ref",
            "--token",
            "--registry-url",
            "--registry",
            "--description",
            "--yes",
            "--dry-run",
        ],
        "self" => &[
            "--deka", "--dsc", "--filter", "-f", "--jobs", "--list", "-l",
        ],
        "wasm" => &["--root"],
        _ => &[],
    }
}

/// Flag names that apply across every command (permissions, `--help`,
/// `--version`, ...). Registered once in `cli::register_global_flags`.
/// Kept as a name list here (not re-exported constants) so this stays a
/// pure documentation concern, independent of registration order.
const GLOBAL_FLAG_NAMES: &[&str] = &[
    "--help",
    "--version",
    "--verbose",
    "--update",
    "--debug",
    "--allow-read",
    "--allow-write",
    "--allow-net",
    "--allow-env",
    "--allow-run",
    "--allow-db",
    "--allow-dynamic",
    "--allow-wasm",
    "--allow-all",
    "--deny-read",
    "--deny-write",
    "--deny-net",
    "--deny-env",
    "--deny-run",
    "--deny-db",
    "--deny-dynamic",
    "--deny-wasm",
    "--no-prompt",
];

/// The global flags section of `deka --help`, deduplicated by name and
/// scoped to flags that are genuinely global. Command-specific flags (like
/// the three formerly-duplicated `--yes` entries) live in their owning
/// command's own help instead — see [`render_command_help`].
pub fn global_flags(registry: &Registry) -> Vec<&FlagSpec> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for flag in registry.flags() {
        if !GLOBAL_FLAG_NAMES.contains(&flag.name) {
            continue;
        }
        if seen.insert(flag.name) {
            out.push(flag);
        }
    }
    out
}

fn find_flag<'a>(registry: &'a Registry, name: &str) -> Option<&'a FlagSpec> {
    registry.flags().iter().find(|flag| flag.name == name)
}

fn find_param<'a>(registry: &'a Registry, name: &str) -> Option<&'a ParamSpec> {
    registry.params().iter().find(|param| param.name == name)
}

/// Render `deka <command> --help`: a usage line, what the command does,
/// its own flags, and — for the handful of commands new users touch
/// first — at least one worked example. Falls back to registry data
/// (summary + subcommands + known flag names) for every other command,
/// so no command is ever left printing the global wall of text (deka#977).
pub fn render_command_help(registry: &Registry, command: &CommandSpec) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(help) = curated_help(command.name) {
        lines.push(format!("Usage: {}", help.usage));
        lines.push(String::new());
        lines.push(help.about.to_string());
        lines.push(String::new());
        render_flags_section(&mut lines, registry, command.name);
        if !help.examples.is_empty() {
            lines.push("Examples:".to_string());
            for (example, note) in help.examples {
                lines.push(format!("  {example}"));
                lines.push(format!("    {note}"));
            }
            lines.push(String::new());
        }
    } else {
        lines.push(format!("Usage: deka {} [options]", command.name));
        lines.push(String::new());
        lines.push(command.summary.to_string());
        lines.push(String::new());
        if !command.subcommands.is_empty() {
            lines.push("Subcommands:".to_string());
            for subcommand in command.subcommands {
                lines.push(format!(
                    "  {} {}\t{}",
                    command.name, subcommand.name, subcommand.summary
                ));
            }
            lines.push(String::new());
        }
        render_flags_section(&mut lines, registry, command.name);
    }

    lines.push("Run `deka --help` to see every command.".to_string());
    lines
}

fn render_flags_section(lines: &mut Vec<String>, registry: &Registry, command_name: &str) {
    let names = command_flag_names(command_name);
    if names.is_empty() {
        return;
    }
    let mut seen = std::collections::HashSet::new();
    let mut printed = false;
    for name in names {
        if !seen.insert(*name) {
            continue;
        }
        if let Some(flag) = find_flag(registry, name) {
            if !printed {
                lines.push("Flags:".to_string());
                printed = true;
            }
            lines.push(format!("  {}\t\t{}", flag.name, flag.description));
        } else if let Some(param) = find_param(registry, name) {
            if !printed {
                lines.push("Flags:".to_string());
                printed = true;
            }
            lines.push(format!("  {} <value>\t{}", param.name, param.description));
        }
    }
    if printed {
        lines.push(String::new());
    }
}

/// Full body of `deka --help`, everything after the ascii banner: usage,
/// version, a Getting Started block (deka#978), the existing
/// category-grouped command list, and a global-only, deduplicated flags
/// section. `cli::help` owns printing the banner and these lines; this
/// function owns building them, to keep `crates/cli/src` inside its
/// composition-only line budget.
pub fn render_global_help(registry: &Registry, version: &str) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Usage: deka [options] [command]".to_string());
    lines.push(format!("deka v{version} - the cloud is a lie"));
    lines.push(String::new());

    let dim = "\x1b[2m";
    let reset = "\x1b[0m";

    // Getting Started comes first: a new user reads top-down, and the
    // commands that take them from nothing to a running page should
    // outrank auth/database/debug just because those sort earlier
    // alphabetically (deka#978).
    let known: std::collections::HashSet<&str> =
        registry.commands().iter().map(|c| c.name).collect();
    lines.push(format!("{dim}Getting Started{reset}"));
    for (name, blurb) in GETTING_STARTED {
        if known.contains(name) {
            lines.push(format!("  {name}\t\t{blurb}"));
        }
    }
    lines.push("  deka <command> --help\t\tshow detailed help for any command".to_string());
    lines.push(String::new());

    let mut grouped: std::collections::BTreeMap<&str, Vec<&CommandSpec>> =
        std::collections::BTreeMap::new();
    for command in registry.commands() {
        grouped.entry(command.category).or_default().push(command);
    }
    for (category, commands) in grouped {
        lines.push(format!("{dim}{category}{reset}"));
        for command in commands {
            lines.push(format!("  {}\t\t{}", command.name, command.summary));
            for subcommand in command.subcommands {
                lines.push(format!(
                    "  {} {}\t{}",
                    command.name, subcommand.name, subcommand.summary
                ));
            }
        }
        lines.push(String::new());
    }

    let flags = global_flags(registry);
    if !flags.is_empty() {
        lines.push(format!("{dim}flags{reset}"));
        for flag in flags {
            lines.push(format!("  {}\t\t{}", flag.name, flag.description));
        }
        lines.push(String::new());
        lines.push(
            "Flags above apply everywhere. Run `deka <command> --help` for a command's own flags."
                .to_string(),
        );
        lines.push(String::new());
    }

    lines
}

/// One line of the "Getting Started" block at the top of `deka --help`:
/// the handful of commands that take a new user from nothing to a running
/// page (deka#978). Every command named here has curated help — see
/// [`curated_help`] — so drilling in with `--help` always works.
pub const GETTING_STARTED: &[(&str, &str)] = &[
    ("init", "scaffold a new project"),
    ("serve", "run it as a local dev server"),
    ("dev", "same, plus hot reload while you edit"),
    ("add", "add a package to your project"),
    ("build", "build your project for production"),
    ("run", "run a single .ds/.dsx file or script"),
];
