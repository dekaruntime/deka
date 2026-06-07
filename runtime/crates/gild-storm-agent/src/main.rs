mod disk;
mod net;
mod process;
mod stress;
mod supervisor;
mod systemd_mask;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "gild-storm-agent",
    version,
    about = "Per-host fault injector for gild-storm"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Process(process::ProcessArgs),
    Net(net::NetArgs),
    Disk(disk::DiskArgs),
    Mem(stress::MemArgs),
    Cpu(stress::CpuArgs),
    Recover,
    #[command(name = "__supervisor", hide = true)]
    Supervisor(SupervisorArgs),
}

#[derive(Debug, Parser)]
struct SupervisorArgs {
    #[arg(long)]
    spec: PathBuf,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Commands::Process(args) => process::run(args),
        Commands::Net(args) => net::run(args),
        Commands::Disk(args) => disk::run(args),
        Commands::Mem(args) => stress::run_mem(args),
        Commands::Cpu(args) => stress::run_cpu(args),
        Commands::Recover => systemd_mask::recover_orphaned_masks(),
        Commands::Supervisor(args) => supervisor::run_supervisor(&args.spec),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_required_commands() {
        for args in [
            vec![
                "gild-storm-agent",
                "process",
                "kill",
                "--service",
                "gild-vault",
                "--signal",
                "SIGTERM",
                "--undo-by",
                "5s",
            ],
            vec![
                "gild-storm-agent",
                "process",
                "start",
                "--service",
                "gild-vault",
            ],
            vec![
                "gild-storm-agent",
                "process",
                "pause",
                "--service",
                "gild-vault",
                "--secs",
                "3",
                "--undo-by",
                "5s",
            ],
            vec![
                "gild-storm-agent",
                "net",
                "partition",
                "--to",
                "demon",
                "--secs",
                "5",
                "--undo-by",
                "10s",
            ],
            vec![
                "gild-storm-agent",
                "net",
                "latency",
                "--to",
                "tailscale0",
                "--ms",
                "50",
                "--jitter",
                "10",
                "--secs",
                "5",
                "--undo-by",
                "10s",
            ],
            vec![
                "gild-storm-agent",
                "net",
                "loss",
                "--to",
                "tailscale0",
                "--pct",
                "5",
                "--secs",
                "5",
                "--undo-by",
                "10s",
            ],
            vec![
                "gild-storm-agent",
                "disk",
                "fill",
                "--path",
                "/tmp",
                "--gb",
                "1",
                "--secs",
                "5",
                "--undo-by",
                "10s",
            ],
            vec![
                "gild-storm-agent",
                "mem",
                "stress",
                "--gb",
                "1",
                "--secs",
                "5",
                "--undo-by",
                "10s",
            ],
            vec![
                "gild-storm-agent",
                "cpu",
                "stress",
                "--pct",
                "80",
                "--secs",
                "5",
                "--undo-by",
                "10s",
            ],
            vec!["gild-storm-agent", "recover"],
        ] {
            Cli::try_parse_from(args).unwrap();
        }
    }

    #[test]
    fn parses_mask_restart_flags() {
        for args in [
            vec![
                "gild-storm-agent",
                "process",
                "kill",
                "--service",
                "gild-vault",
                "--signal",
                "SIGKILL",
                "--undo-by",
                "5s",
                "--mask-restart",
            ],
            vec![
                "gild-storm-agent",
                "process",
                "pause",
                "--service",
                "gild-vault",
                "--secs",
                "3",
                "--undo-by",
                "5s",
                "--mask-restart",
            ],
        ] {
            Cli::try_parse_from(args).unwrap();
        }
    }

    #[test]
    fn rejects_zero_undo_by() {
        let err = Cli::try_parse_from([
            "gild-storm-agent",
            "process",
            "kill",
            "--service",
            "gild-vault",
            "--signal",
            "SIGTERM",
            "--undo-by",
            "0s",
        ])
        .unwrap_err();
        assert!(err.to_string().contains("--undo-by must be greater than 0"));
    }

    #[test]
    fn requires_undo_by_for_reversible_commands() {
        for args in [
            vec![
                "gild-storm-agent",
                "process",
                "pause",
                "--service",
                "gild-vault",
                "--secs",
                "3",
            ],
            vec![
                "gild-storm-agent",
                "net",
                "partition",
                "--to",
                "demon",
                "--secs",
                "5",
            ],
            vec![
                "gild-storm-agent",
                "disk",
                "fill",
                "--path",
                "/tmp",
                "--gb",
                "1",
                "--secs",
                "5",
            ],
            vec![
                "gild-storm-agent",
                "mem",
                "stress",
                "--gb",
                "1",
                "--secs",
                "5",
            ],
            vec![
                "gild-storm-agent",
                "cpu",
                "stress",
                "--pct",
                "80",
                "--secs",
                "5",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
