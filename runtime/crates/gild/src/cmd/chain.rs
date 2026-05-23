use clap::{Args, Subcommand};

use crate::Result;

#[derive(Debug, Args)]
pub struct ChainArgs {
    #[command(subcommand)]
    command: ChainCommand,
}

#[derive(Debug, Subcommand)]
enum ChainCommand {
    /// Run a named orchestration policy.
    Run {
        #[arg(long)]
        policy: String,
    },
}

pub async fn run(_args: ChainArgs) -> Result<()> {
    println!("not yet implemented in this skeleton PR");
    Ok(())
}
