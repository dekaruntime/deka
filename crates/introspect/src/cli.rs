use core::{CommandSpec, Context, FlagSpec, ParamSpec, Registry, SubcommandSpec};
use std::path::PathBuf;

const COMMAND: CommandSpec = CommandSpec {
    name: "introspect",
    category: "debug",
    summary: "inspect deka-runtime scheduler state",
    aliases: &[],
    subcommands: &[
        SubcommandSpec {
            name: "top",
            summary: "show top isolates",
            aliases: &[],
            handler: cmd_top,
        },
        SubcommandSpec {
            name: "workers",
            summary: "show worker stats",
            aliases: &[],
            handler: cmd_workers,
        },
        SubcommandSpec {
            name: "inspect",
            summary: "inspect a specific isolate",
            aliases: &[],
            handler: cmd_inspect,
        },
        SubcommandSpec {
            name: "kill",
            summary: "kill a specific isolate",
            aliases: &[],
            handler: cmd_kill,
        },
    ],
    handler: cmd_default,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--archive",
        aliases: &[],
        description: "show archived request history",
    });
    registry.add_flag(FlagSpec {
        name: "--json",
        aliases: &[],
        description: "output JSON format",
    });
    registry.add_param(ParamSpec {
        name: "--runtime",
        description: "runtime URL (default: http://localhost:8530)",
    });
    registry.add_param(ParamSpec {
        name: "-r",
        description: "runtime URL (default: http://localhost:8530)",
    });
    registry.add_param(ParamSpec {
        name: "--sort",
        description: "sort by cpu|memory|requests (top command)",
    });
    registry.add_param(ParamSpec {
        name: "-s",
        description: "sort by cpu|memory|requests (top command)",
    });
    registry.add_param(ParamSpec {
        name: "--limit",
        description: "limit number of rows (top command)",
    });
    registry.add_param(ParamSpec {
        name: "-l",
        description: "limit number of rows (top command)",
    });
}

fn get_ui_path(filename: &str) -> PathBuf {
    // Get the path to the introspect crate's ui directory
    // This assumes the crate is in deka/crates/introspect
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("src")
        .join("ui")
        .join(filename)
}

// Default command - launch the TUI
pub fn cmd_default(context: &Context) {
    let ui_path = get_ui_path("introspect-ui.tsx");

    if !ui_path.exists() {
        stdio::error(
            "introspect",
            &format!("TUI file not found: {}", ui_path.display()),
        );
        std::process::exit(1);
    }

    // Build arguments for the TUI
    let mut args = Vec::new();

    if let Some(runtime) = context
        .args
        .params
        .get("--runtime")
        .or_else(|| context.args.params.get("-r"))
    {
        args.push("--runtime".to_string());
        args.push(runtime.clone());
    }

    if context.args.flags.contains_key("--archive") {
        args.push("--archive".to_string());
    }

    // Set up environment for the runtime to execute the TUI
    unsafe {
        if !args.is_empty() {
            let args_json = serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string());
            std::env::set_var("DEKA_ARGS", args_json);
        }
        std::env::set_var("HANDLER_PATH", ui_path.to_string_lossy().to_string());
        std::env::set_var("DEKA_JSX_IMPORT_SOURCE", "react");
    }

    // Execute the TUI via runtime
    runtime::run(context);
}

// Top subcommand
pub fn cmd_top(context: &Context) {
    let cli_path = get_ui_path("cli.ts");

    if !cli_path.exists() {
        stdio::error(
            "introspect",
            &format!("cli.ts not found: {}", cli_path.display()),
        );
        std::process::exit(1);
    }

    // Build arguments for introspectTop
    let mut args = vec!["top".to_string()];

    if let Some(runtime) = context
        .args
        .params
        .get("--runtime")
        .or_else(|| context.args.params.get("-r"))
    {
        args.push("--runtime".to_string());
        args.push(runtime.clone());
    }

    if let Some(sort) = context
        .args
        .params
        .get("--sort")
        .or_else(|| context.args.params.get("-s"))
    {
        args.push("--sort".to_string());
        args.push(sort.clone());
    }

    if let Some(limit) = context
        .args
        .params
        .get("--limit")
        .or_else(|| context.args.params.get("-l"))
    {
        args.push("--limit".to_string());
        args.push(limit.clone());
    }

    if context.args.flags.contains_key("--json") {
        args.push("--json".to_string());
    }

    execute_introspect_command(&cli_path, args, context);
}

// Workers subcommand
pub fn cmd_workers(context: &Context) {
    let cli_path = get_ui_path("cli.ts");

    if !cli_path.exists() {
        stdio::error(
            "introspect",
            &format!("cli.ts not found: {}", cli_path.display()),
        );
        std::process::exit(1);
    }

    let mut args = vec!["workers".to_string()];

    if let Some(runtime) = context
        .args
        .params
        .get("--runtime")
        .or_else(|| context.args.params.get("-r"))
    {
        args.push("--runtime".to_string());
        args.push(runtime.clone());
    }

    if context.args.flags.contains_key("--json") {
        args.push("--json".to_string());
    }

    execute_introspect_command(&cli_path, args, context);
}

// Inspect subcommand
pub fn cmd_inspect(context: &Context) {
    let cli_path = get_ui_path("cli.ts");

    if !cli_path.exists() {
        stdio::error(
            "introspect",
            &format!("cli.ts not found: {}", cli_path.display()),
        );
        std::process::exit(1);
    }

    let handler = context.args.positionals.get(0).cloned().unwrap_or_default();
    if handler.is_empty() {
        stdio::error("introspect", "inspect requires a handler argument");
        std::process::exit(1);
    }

    let mut args = vec!["inspect".to_string(), handler];

    if let Some(runtime) = context
        .args
        .params
        .get("--runtime")
        .or_else(|| context.args.params.get("-r"))
    {
        args.push("--runtime".to_string());
        args.push(runtime.clone());
    }

    if context.args.flags.contains_key("--json") {
        args.push("--json".to_string());
    }

    execute_introspect_command(&cli_path, args, context);
}

// Kill subcommand
pub fn cmd_kill(context: &Context) {
    let cli_path = get_ui_path("cli.ts");

    if !cli_path.exists() {
        stdio::error(
            "introspect",
            &format!("cli.ts not found: {}", cli_path.display()),
        );
        std::process::exit(1);
    }

    let handler = context.args.positionals.get(0).cloned().unwrap_or_default();
    if handler.is_empty() {
        stdio::error("introspect", "kill requires a handler argument");
        std::process::exit(1);
    }

    let mut args = vec!["kill".to_string(), handler];

    if let Some(runtime) = context
        .args
        .params
        .get("--runtime")
        .or_else(|| context.args.params.get("-r"))
    {
        args.push("--runtime".to_string());
        args.push(runtime.clone());
    }

    execute_introspect_command(&cli_path, args, context);
}

fn execute_introspect_command(wrapper_path: &PathBuf, args: Vec<String>, context: &Context) {
    unsafe {
        let args_json = serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string());
        std::env::set_var("DEKA_ARGS", args_json);
        std::env::set_var("HANDLER_PATH", wrapper_path.to_string_lossy().to_string());
    }

    runtime::run(context);
}
