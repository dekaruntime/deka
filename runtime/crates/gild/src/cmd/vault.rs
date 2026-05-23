use clap::{Args, Subcommand};

use crate::Result;

#[derive(Debug, Args)]
pub struct VaultArgs {
    #[command(subcommand)]
    command: VaultCommand,
}

#[derive(Debug, Subcommand)]
enum VaultCommand {
    /// Get a secret value.
    Get { key: String },
}

pub async fn run(_args: VaultArgs) -> Result<()> {
    println!("not yet implemented in this skeleton PR");
    Ok(())
}
