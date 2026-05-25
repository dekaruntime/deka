use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::process::{ensure_undo_by_at_least, parse_nonzero_duration};
use crate::supervisor::{self, UndoAction};

#[derive(Debug, Args)]
pub struct DiskArgs {
    #[command(subcommand)]
    pub command: DiskCommand,
}

#[derive(Debug, Subcommand)]
pub enum DiskCommand {
    Fill(FillArgs),
}

#[derive(Debug, Args)]
pub struct FillArgs {
    #[arg(long)]
    pub path: PathBuf,
    #[arg(long)]
    pub gb: u64,
    #[arg(long)]
    pub secs: u64,
    #[arg(long, value_parser = parse_nonzero_duration)]
    pub undo_by: Duration,
}

pub fn run(args: DiskArgs) -> Result<()> {
    match args.command {
        DiskCommand::Fill(args) => {
            ensure_undo_by_at_least(args.undo_by, args.secs, "disk fill")?;
            std::fs::create_dir_all(&args.path)?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let file = args
                .path
                .join(format!("gild-storm-fill-{}-{now}.bin", std::process::id()));
            supervisor::spawn_supervisor(
                args.undo_by,
                UndoAction::RemoveFile { path: file.clone() },
            )?;
            let count = (args.gb * 1024).to_string();
            let of = format!("of={}", file.display());
            let status = std::process::Command::new("dd")
                .args(["if=/dev/zero", &of, "bs=1m", &format!("count={count}")])
                .status()?;
            if !status.success() {
                bail!("dd exited with {status}");
            }
            Ok(())
        }
    }
}
