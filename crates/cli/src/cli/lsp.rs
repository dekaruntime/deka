use core::{CommandSpec, Context, FlagSpec, Registry};

const COMMAND: CommandSpec = CommandSpec {
    name: "lsp",
    category: "tooling",
    summary: "run the DekaScript language server",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--stdio",
        aliases: &[],
        description: "run the language server over stdio",
    });
}

pub fn cmd(_context: &Context) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            stdio::error("cli", &format!("failed to initialize lsp runtime: {}", err));
            std::process::exit(1);
        }
    };

    let status = runtime.block_on(async { dekascript_lsp::run_stdio().await });
    if let Err(err) = status {
        stdio::error("cli", &format!("failed to start DekaScript lsp: {}", err));
        std::process::exit(1);
    }
}
