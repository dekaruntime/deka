use clap::{Args, Subcommand};
use gild_chain::{Policy, StateStore, DEFAULT_STATE_DB};
use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::Result;

const DEFAULT_LOG_FILE: &str = "/var/log/gild-chain.log";
const DEFAULT_PID_FILE: &str = "/var/lib/gild-chain/gild-chain.pid";
const DEFAULT_CONFIG_FILE: &str = "/etc/gild/chain.toml";
const SUPPORTED_POLICY: &str = "default-flow";

#[derive(Debug, Args)]
pub struct ChainArgs {
    #[command(subcommand)]
    command: ChainCommand,
}

#[derive(Debug, Subcommand)]
enum ChainCommand {
    /// Start the gild-chain daemon.
    Run(RunArgs),
    /// Show current chain state from sqlite.
    Status(StatusArgs),
    /// Stop a running gild-chain daemon.
    Stop(StopArgs),
}

#[derive(Debug, Args, Clone)]
pub struct RunArgs {
    /// Chain policy to run.
    #[arg(long, required_unless_present = "list_policies")]
    policy: Option<String>,
    /// Print supported policies and exit.
    #[arg(long)]
    list_policies: bool,
    /// sqlite state database path.
    #[arg(long, default_value = DEFAULT_STATE_DB)]
    state_db: PathBuf,
    /// Pulse SSE subscribe URL. Defaults to GILD_CHAIN_PULSE_URL or config.
    #[arg(long)]
    pulse_url: Option<String>,
    /// Keep gild-chain attached to this terminal.
    #[arg(long)]
    foreground: bool,
    /// Background daemon log file.
    #[arg(long, default_value = DEFAULT_LOG_FILE)]
    log_file: PathBuf,
    /// PID file used by status/stop.
    #[arg(long, default_value = DEFAULT_PID_FILE)]
    pid_file: PathBuf,
}

#[derive(Debug, Args, Clone)]
pub struct StatusArgs {
    /// sqlite state database path.
    #[arg(long, default_value = DEFAULT_STATE_DB)]
    state_db: PathBuf,
    /// PID file written by `gild chain run`.
    #[arg(long, default_value = DEFAULT_PID_FILE)]
    pid_file: PathBuf,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args, Clone)]
pub struct StopArgs {
    /// PID file written by `gild chain run`.
    #[arg(long, default_value = DEFAULT_PID_FILE)]
    pid_file: PathBuf,
}

pub async fn run(args: ChainArgs) -> Result<()> {
    match args.command {
        ChainCommand::Run(args) => run_with_spawner(args, &RealSpawner).await,
        ChainCommand::Status(args) => status(args),
        ChainCommand::Stop(args) => stop(args),
    }
}

trait ChainSpawner {
    fn spawn(&self, command: ChainDaemonCommand, foreground: bool, log_file: &Path) -> Result<u32>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainDaemonCommand {
    bin: String,
    args: Vec<String>,
    state_db: PathBuf,
}

struct RealSpawner;

impl ChainSpawner for RealSpawner {
    fn spawn(&self, command: ChainDaemonCommand, foreground: bool, log_file: &Path) -> Result<u32> {
        let mut child = Command::new(&command.bin);
        child.args(&command.args);
        child.env("GILD_CHAIN_STATE_DB", &command.state_db);
        if foreground {
            child
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        } else {
            if let Some(parent) = log_file.parent() {
                fs::create_dir_all(parent)?;
            }
            let stdout = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_file)?;
            let stderr = stdout.try_clone()?;
            child
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr));
        }
        child.spawn().map(|child| child.id()).map_err(Into::into)
    }
}

async fn run_with_spawner(args: RunArgs, spawner: &dyn ChainSpawner) -> Result<()> {
    if args.list_policies {
        for policy in Policy::supported_cli_policies() {
            println!("{policy}");
        }
        return Ok(());
    }

    let policy = validate_policy(args.policy.as_deref())?;
    let pulse_url = resolve_pulse_url(args.pulse_url.as_deref())?;
    let command = build_daemon_command(policy, &args.state_db, &pulse_url);
    let pid = spawner.spawn(command, args.foreground, &args.log_file)?;
    write_pid_file(&args.pid_file, pid)?;
    println!("gild-chain pid: {pid}");
    println!("log: {}", args.log_file.display());
    Ok(())
}

fn status(args: StatusArgs) -> Result<()> {
    let store = StateStore::open(&args.state_db)?;
    let summary = store.summary()?;
    let active = store.list_active()?;
    let pid = read_pid_file(&args.pid_file).ok().flatten();
    let running = pid.is_some_and(process_exists);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "state_db": args.state_db,
                "pid": pid,
                "running": running,
                "summary": summary,
                "active": active,
            }))?
        );
        return Ok(());
    }

    println!("state_db: {}", args.state_db.display());
    match pid {
        Some(pid) if running => println!("daemon: running pid={pid}"),
        Some(pid) => println!("daemon: not running stale_pid={pid}"),
        None => println!("daemon: not running"),
    }
    println!("active runs: {}", summary.active_runs);
    println!("completed flows: {}", summary.completed_flows);
    println!("failed flows: {}", summary.failed_flows);
    println!("pending merges: {}", summary.pending_merges);
    if !active.is_empty() {
        println!("active:");
        for chain in active {
            println!(
                "  {} {} {} {}",
                chain.id, chain.policy, chain.status, chain.dispatch_agent
            );
        }
    }
    Ok(())
}

fn stop(args: StopArgs) -> Result<()> {
    let Some(pid) = read_pid_file(&args.pid_file)? else {
        println!("not running");
        return Ok(());
    };
    if !process_exists(pid) {
        let _ = fs::remove_file(&args.pid_file);
        println!("not running");
        return Ok(());
    }

    send_signal(pid, libc::SIGTERM)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !process_exists(pid) {
            let _ = fs::remove_file(&args.pid_file);
            println!("stopped");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    send_signal(pid, libc::SIGKILL)?;
    let _ = fs::remove_file(&args.pid_file);
    println!("stopped");
    Ok(())
}

fn build_daemon_command(policy: &str, state_db: &Path, pulse_url: &str) -> ChainDaemonCommand {
    ChainDaemonCommand {
        bin: std::env::var("GILD_CHAIN_BIN").unwrap_or_else(|_| "gild-chain".to_string()),
        args: vec![
            "daemon".to_string(),
            "--policy".to_string(),
            policy.to_string(),
            "--state-db".to_string(),
            state_db.display().to_string(),
            "--pulse-url".to_string(),
            pulse_url.to_string(),
        ],
        state_db: state_db.to_path_buf(),
    }
}

fn validate_policy(policy: Option<&str>) -> Result<&'static str> {
    match policy {
        Some(SUPPORTED_POLICY) => Ok(SUPPORTED_POLICY),
        Some(other) => Err(format!(
            "unsupported chain policy {other:?}; supported policies: {SUPPORTED_POLICY}"
        )
        .into()),
        None => Err("--policy is required".into()),
    }
}

fn resolve_pulse_url(explicit: Option<&str>) -> Result<String> {
    if let Some(url) = explicit {
        return Ok(url.to_string());
    }
    if let Ok(url) = std::env::var("GILD_CHAIN_PULSE_URL") {
        return Ok(url);
    }
    if let Ok(url) = std::env::var("PULSE_URL") {
        return Ok(url);
    }
    let config_path =
        std::env::var("GILD_CHAIN_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_FILE.to_string());
    if let Ok(config) = fs::read_to_string(&config_path) {
        let config: ChainConfig = toml::from_str(&config)?;
        if let Some(url) = config.pulse_url {
            return Ok(url);
        }
    }
    Err("missing --pulse-url, GILD_CHAIN_PULSE_URL, or pulse_url in /etc/gild/chain.toml".into())
}

fn write_pid_file(path: &Path, pid: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{pid}\n"))?;
    Ok(())
}

fn read_pid_file(path: &Path) -> Result<Option<i32>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents.trim().parse()?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn process_exists(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn send_signal(pid: i32, signal: i32) -> Result<()> {
    let rc = unsafe { libc::kill(pid, signal) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[derive(Debug, Deserialize)]
struct ChainConfig {
    pulse_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn validates_only_default_flow() {
        assert_eq!(
            validate_policy(Some("default-flow")).unwrap(),
            "default-flow"
        );
        assert!(validate_policy(Some("manual-merge")).is_err());
    }

    #[test]
    fn builds_daemon_command_args() {
        let command = build_daemon_command(
            "default-flow",
            Path::new("/tmp/state.db"),
            "http://pulse/v1/subscribe",
        );
        assert_eq!(command.bin, "gild-chain");
        assert_eq!(
            command.args,
            [
                "daemon",
                "--policy",
                "default-flow",
                "--state-db",
                "/tmp/state.db",
                "--pulse-url",
                "http://pulse/v1/subscribe"
            ]
        );
    }

    #[tokio::test]
    async fn run_uses_mock_spawner() {
        let dir = std::env::temp_dir().join(format!(
            "gild-chain-run-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("chain.pid");
        let state_db = dir.join("state.db");
        let log_file = dir.join("chain.log");
        let spawner = MockSpawner::default();

        run_with_spawner(
            RunArgs {
                policy: Some("default-flow".into()),
                list_policies: false,
                state_db: state_db.clone(),
                pulse_url: Some("http://pulse/v1/subscribe".into()),
                foreground: false,
                log_file,
                pid_file: pid_file.clone(),
            },
            &spawner,
        )
        .await
        .unwrap();

        let captured = spawner.commands.borrow();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].state_db, state_db);
        assert_eq!(read_pid_file(&pid_file).unwrap(), Some(4242));
    }

    #[test]
    fn status_reads_sqlite_summary() {
        let dir = std::env::temp_dir().join(format!(
            "gild-chain-status-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let state_db = dir.join("state.db");
        let store = StateStore::open(&state_db).unwrap();
        let chain =
            gild_chain::new_chain(Policy::DefaultFlow, "agent-khalid".into(), "do work".into());
        store.insert_chain(&chain).unwrap();
        drop(store);

        status(StatusArgs {
            state_db,
            pid_file: dir.join("missing.pid"),
            json: true,
        })
        .unwrap();
    }

    #[derive(Default)]
    struct MockSpawner {
        commands: RefCell<Vec<ChainDaemonCommand>>,
    }

    impl ChainSpawner for MockSpawner {
        fn spawn(
            &self,
            command: ChainDaemonCommand,
            _foreground: bool,
            _log_file: &Path,
        ) -> Result<u32> {
            self.commands.borrow_mut().push(command);
            Ok(4242)
        }
    }
}
