use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::process::{ensure_undo_by_at_least, parse_nonzero_duration};
use crate::supervisor::{self, UndoAction};

#[derive(Debug, Args)]
pub struct NetArgs {
    #[command(subcommand)]
    pub command: NetCommand,
}

#[derive(Debug, Subcommand)]
pub enum NetCommand {
    Partition(PartitionArgs),
    Latency(LatencyArgs),
    Loss(LossArgs),
}

#[derive(Debug, Args)]
pub struct PartitionArgs {
    #[arg(long)]
    pub to: String,
    #[arg(long)]
    pub secs: u64,
    #[arg(long, value_parser = parse_nonzero_duration)]
    pub undo_by: Duration,
}

#[derive(Debug, Args)]
pub struct LatencyArgs {
    #[arg(long)]
    pub to: String,
    #[arg(long)]
    pub ms: u64,
    #[arg(long)]
    pub jitter: u64,
    #[arg(long)]
    pub secs: u64,
    #[arg(long, value_parser = parse_nonzero_duration)]
    pub undo_by: Duration,
}

#[derive(Debug, Args)]
pub struct LossArgs {
    #[arg(long)]
    pub to: String,
    #[arg(long)]
    pub pct: f64,
    #[arg(long)]
    pub secs: u64,
    #[arg(long, value_parser = parse_nonzero_duration)]
    pub undo_by: Duration,
}

pub fn run(args: NetArgs) -> Result<()> {
    match args.command {
        NetCommand::Partition(args) => {
            ensure_undo_by_at_least(args.undo_by, args.secs, "net partition")?;
            supervisor::spawn_supervisor(args.undo_by, UndoAction::TailscaleUp)?;
            run_status(std::process::Command::new("tailscale").arg("down"))
        }
        NetCommand::Latency(args) => {
            ensure_undo_by_at_least(args.undo_by, args.secs, "net latency")?;
            supervisor::spawn_supervisor(
                args.undo_by,
                UndoAction::TcNetemDel {
                    peer: args.to.clone(),
                },
            )?;
            run_status(std::process::Command::new("tc").args([
                "qdisc",
                "replace",
                "dev",
                &args.to,
                "root",
                "netem",
                "delay",
                &format!("{}ms", args.ms),
                &format!("{}ms", args.jitter),
            ]))
        }
        NetCommand::Loss(args) => {
            ensure_undo_by_at_least(args.undo_by, args.secs, "net loss")?;
            supervisor::spawn_supervisor(
                args.undo_by,
                UndoAction::TcNetemDel {
                    peer: args.to.clone(),
                },
            )?;
            run_status(std::process::Command::new("tc").args([
                "qdisc",
                "replace",
                "dev",
                &args.to,
                "root",
                "netem",
                "loss",
                &format!("{}%", args.pct),
            ]))
        }
    }
}

fn run_status(cmd: &mut std::process::Command) -> Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        bail!("command exited with {status}");
    }
    Ok(())
}
