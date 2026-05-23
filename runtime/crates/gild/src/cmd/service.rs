use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::Result;

#[derive(Debug, Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Link provider credentials to an agent.
    Link {
        #[arg(long)]
        agent: String,
        #[arg(long)]
        provider: Provider,
        #[arg(long)]
        from: PathBuf,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum Provider {
    Opencode,
    Codex,
    Claude,
}

pub async fn run(_args: ServiceArgs) -> Result<()> {
    println!("not yet implemented in this skeleton PR");
    Ok(())
}
