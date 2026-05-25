use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::process::{ensure_undo_by_at_least, parse_nonzero_duration};
use crate::supervisor::{self, UndoAction};

#[derive(Debug, Args)]
pub struct MemArgs {
    #[command(subcommand)]
    pub command: MemCommand,
}

#[derive(Debug, Subcommand)]
pub enum MemCommand {
    Stress(MemStressArgs),
}

#[derive(Debug, Args)]
pub struct CpuArgs {
    #[command(subcommand)]
    pub command: CpuCommand,
}

#[derive(Debug, Subcommand)]
pub enum CpuCommand {
    Stress(CpuStressArgs),
}

#[derive(Debug, Args)]
pub struct MemStressArgs {
    #[arg(long)]
    pub gb: u64,
    #[arg(long)]
    pub secs: u64,
    #[arg(long, value_parser = parse_nonzero_duration)]
    pub undo_by: Duration,
}

#[derive(Debug, Args)]
pub struct CpuStressArgs {
    #[arg(long)]
    pub pct: u64,
    #[arg(long)]
    pub secs: u64,
    #[arg(long, value_parser = parse_nonzero_duration)]
    pub undo_by: Duration,
}

pub fn run_mem(args: MemArgs) -> Result<()> {
    match args.command {
        MemCommand::Stress(args) => {
            ensure_undo_by_at_least(args.undo_by, args.secs, "mem stress")?;
            let mut child = std::process::Command::new("stress-ng")
                .args([
                    "--vm",
                    "1",
                    "--vm-bytes",
                    &format!("{}G", args.gb),
                    "--timeout",
                    &format!("{}s", args.secs),
                ])
                .spawn()?;
            supervisor::spawn_supervisor(
                args.undo_by,
                UndoAction::KillPid {
                    pid: child.id(),
                    signal: "TERM".to_string(),
                },
            )?;
            let status = child.wait()?;
            if !status.success() {
                bail!("stress-ng exited with {status}");
            }
            Ok(())
        }
    }
}

pub fn run_cpu(args: CpuArgs) -> Result<()> {
    match args.command {
        CpuCommand::Stress(args) => {
            ensure_undo_by_at_least(args.undo_by, args.secs, "cpu stress")?;
            let load = args.pct.min(100).to_string();
            let mut child = std::process::Command::new("stress-ng")
                .args([
                    "--cpu",
                    "0",
                    "--cpu-load",
                    &load,
                    "--timeout",
                    &format!("{}s", args.secs),
                ])
                .spawn()?;
            supervisor::spawn_supervisor(
                args.undo_by,
                UndoAction::KillPid {
                    pid: child.id(),
                    signal: "TERM".to_string(),
                },
            )?;
            let status = child.wait()?;
            if !status.success() {
                bail!("stress-ng exited with {status}");
            }
            Ok(())
        }
    }
}
