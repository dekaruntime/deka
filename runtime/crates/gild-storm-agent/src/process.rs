use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Args, Subcommand, ValueEnum};

use crate::supervisor::{self, UndoAction};

#[derive(Debug, Args)]
pub struct ProcessArgs {
    #[command(subcommand)]
    pub command: ProcessCommand,
}

#[derive(Debug, Subcommand)]
pub enum ProcessCommand {
    Kill(KillArgs),
    Start(ServiceArgs),
    Pause(PauseArgs),
}

#[derive(Debug, Args)]
pub struct ServiceArgs {
    #[arg(long)]
    pub service: String,
}

#[derive(Debug, Args)]
pub struct KillArgs {
    #[arg(long)]
    pub service: String,
    #[arg(long, value_enum)]
    pub signal: Signal,
    #[arg(long, value_parser = parse_nonzero_duration)]
    pub undo_by: Duration,
}

#[derive(Debug, Args)]
pub struct PauseArgs {
    #[arg(long)]
    pub service: String,
    #[arg(long)]
    pub secs: u64,
    #[arg(long, value_parser = parse_nonzero_duration)]
    pub undo_by: Duration,
}

#[derive(Debug, Clone, ValueEnum)]
#[value(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Signal {
    Sigterm,
    Sigkill,
}

impl Signal {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Sigterm => "SIGTERM",
            Self::Sigkill => "SIGKILL",
        }
    }
}

pub fn run(args: ProcessArgs) -> Result<()> {
    match args.command {
        ProcessCommand::Kill(args) => {
            supervisor::spawn_supervisor(
                args.undo_by,
                UndoAction::SystemctlStart {
                    service: args.service.clone(),
                },
            )?;
            run_systemctl(&["kill", "-s", args.signal.as_str(), &args.service])
        }
        ProcessCommand::Start(args) => {
            supervisor::spawn_supervisor(Duration::from_secs(1), UndoAction::Noop)?;
            run_systemctl(&["start", &args.service])
        }
        ProcessCommand::Pause(args) => {
            ensure_undo_by_at_least(args.undo_by, args.secs, "process pause")?;
            supervisor::spawn_supervisor(
                args.undo_by,
                UndoAction::SignalService {
                    service: args.service.clone(),
                    signal: "SIGCONT".to_string(),
                },
            )?;
            run_systemctl(&["kill", "-s", "SIGSTOP", &args.service])
        }
    }
}

pub(crate) fn parse_nonzero_duration(value: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(value).map_err(|err| err.to_string())?;
    if duration.is_zero() {
        return Err("--undo-by must be greater than 0".to_string());
    }
    Ok(duration)
}

pub(crate) fn ensure_undo_by_at_least(undo_by: Duration, secs: u64, command: &str) -> Result<()> {
    if undo_by < Duration::from_secs(secs) {
        bail!("--undo-by must be >= --secs for {command}");
    }
    Ok(())
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()?;
    if !status.success() {
        bail!("systemctl {} exited with {status}", args.join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_undo_by_shorter_than_operation_window() {
        let err = ensure_undo_by_at_least(Duration::from_secs(4), 5, "process pause").unwrap_err();
        assert!(err.to_string().contains("--undo-by must be >= --secs"));
    }
}
