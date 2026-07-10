use anyhow::anyhow;
use clap::{Args, Subcommand};
use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::chain_core::{
    apply_event, event_name, subscribe_once, Action, Policy, PulseEvent, StateStore,
    DEFAULT_STATE_DB,
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
    /// Start the chain daemon.
    Run(RunArgs),
    /// Show current chain state from sqlite.
    Status(StatusArgs),
    /// Stop a running chain daemon.
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
    /// Dispatcher API URL for review requests.
    #[arg(long)]
    dispatcher_url: Option<String>,
    /// Git server API URL for merge requests.
    #[arg(long)]
    git_api_url: Option<String>,
    /// Git server API token for merge requests.
    #[arg(long)]
    git_token: Option<String>,
    /// Keep the chain daemon attached to this terminal.
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

#[derive(Debug, Args, Clone)]
pub struct DaemonArgs {
    /// Chain policy to run.
    #[arg(long, default_value = SUPPORTED_POLICY)]
    policy: String,
    /// sqlite state database path.
    #[arg(long, default_value = DEFAULT_STATE_DB)]
    state_db: PathBuf,
    /// Pulse SSE subscribe URL.
    #[arg(long)]
    pulse_url: String,
    /// Dispatcher API URL for review requests.
    #[arg(long)]
    dispatcher_url: Option<String>,
    /// Git server API URL for merge requests.
    #[arg(long)]
    git_api_url: Option<String>,
    /// Git server API token for merge requests.
    #[arg(long)]
    git_token: Option<String>,
}

pub async fn daemon(args: DaemonArgs) -> Result<()> {
    let policy = validate_policy(Some(&args.policy))?;
    if policy != SUPPORTED_POLICY {
        return Err(format!("unsupported chain policy {policy:?}").into());
    }
    let store = StateStore::open(&args.state_db)?;
    let executor = ActionExecutor {
        dispatcher_url: args.dispatcher_url,
        git_api_url: args.git_api_url,
        git_token: args
            .git_token
            .or_else(|| std::env::var("GILD_CHAIN_GIT_TOKEN").ok()),
    };
    subscribe_forever(&store, &args.pulse_url, &executor)
        .await
        .map_err(|err| err.into_boxed_dyn_error())
}

trait ChainSpawner {
    fn spawn(&self, command: ChainDaemonCommand, foreground: bool, log_file: &Path) -> Result<u32>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainDaemonCommand {
    bin: PathBuf,
    args: Vec<String>,
}

struct RealSpawner;

impl ChainSpawner for RealSpawner {
    fn spawn(&self, command: ChainDaemonCommand, foreground: bool, log_file: &Path) -> Result<u32> {
        let mut child = Command::new(&command.bin);
        child.args(&command.args);
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
    if args.foreground {
        return daemon(DaemonArgs {
            policy: policy.to_string(),
            state_db: args.state_db,
            pulse_url,
            dispatcher_url: args.dispatcher_url,
            git_api_url: args.git_api_url,
            git_token: args.git_token,
        })
        .await;
    }

    let command = build_daemon_command(
        policy,
        &args.state_db,
        &pulse_url,
        args.dispatcher_url.as_deref(),
        args.git_api_url.as_deref(),
        args.git_token.as_deref(),
    )?;
    let pid = spawner.spawn(command, args.foreground, &args.log_file)?;
    write_pid_file(&args.pid_file, pid)?;
    println!("gild chain pid: {pid}");
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

fn build_daemon_command(
    policy: &str,
    state_db: &Path,
    pulse_url: &str,
    dispatcher_url: Option<&str>,
    git_api_url: Option<&str>,
    git_token: Option<&str>,
) -> Result<ChainDaemonCommand> {
    let mut args = vec![
        "chain-daemon".to_string(),
        "--policy".to_string(),
        policy.to_string(),
        "--state-db".to_string(),
        state_db.display().to_string(),
        "--pulse-url".to_string(),
        pulse_url.to_string(),
    ];
    if let Some(dispatcher_url) = dispatcher_url {
        args.extend(["--dispatcher-url".to_string(), dispatcher_url.to_string()]);
    }
    if let Some(git_api_url) = git_api_url {
        args.extend(["--git-api-url".to_string(), git_api_url.to_string()]);
    }
    if let Some(git_token) = git_token {
        args.extend(["--git-token".to_string(), git_token.to_string()]);
    }
    Ok(ChainDaemonCommand {
        bin: std::env::current_exe()?,
        args,
    })
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

async fn subscribe_forever(
    store: &StateStore,
    pulse_url: &str,
    executor: &ActionExecutor,
) -> anyhow::Result<()> {
    loop {
        if let Err(err) = subscribe_once(pulse_url, None, |event| {
            handle_event(store, executor, event)
        })
        .await
        {
            eprintln!("gild chain pulse subscription error: {err:#}");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

fn handle_event(
    store: &StateStore,
    executor: &ActionExecutor,
    event: PulseEvent,
) -> anyhow::Result<()> {
    let Some(mut chain) = store.find_chain_for_event(&event)? else {
        eprintln!("ignoring event for unknown chain: {}", event_name(&event));
        return Ok(());
    };
    let actions = apply_event(&mut chain, &event);
    store.save_chain(&chain)?;
    store.record_event(&chain.id, &event, &actions)?;
    for action in actions {
        executor.emit_action(&action)?;
    }
    Ok(())
}

struct ActionExecutor {
    dispatcher_url: Option<String>,
    git_api_url: Option<String>,
    git_token: Option<String>,
}

impl ActionExecutor {
    fn emit_action(&self, action: &Action) -> anyhow::Result<()> {
        match action {
            Action::RequestReview {
                reviewer,
                repo,
                pr_number,
            } => self.request_review(reviewer, repo.as_deref(), *pr_number)?,
            Action::MergePullRequest { repo, pr_number } => {
                self.merge_pull_request(repo.as_deref(), *pr_number)?
            }
            Action::Log(message) => println!("note: {message}"),
        }
        Ok(())
    }

    fn request_review(
        &self,
        reviewer: &str,
        repo: Option<&str>,
        pr_number: i64,
    ) -> anyhow::Result<()> {
        let dispatcher_url = self
            .dispatcher_url
            .as_ref()
            .ok_or_else(|| anyhow!("missing --dispatcher-url for review request"))?;
        let task = match repo {
            Some(repo) => format!("Review {repo} PR #{pr_number}"),
            None => format!("Review PR #{pr_number}"),
        };
        let response = reqwest::blocking::Client::new()
            .post(format!("{}/task", dispatcher_url.trim_end_matches('/')))
            .json(&serde_json::json!({
                "agent": reviewer,
                "task": task,
                "runtime": "codex"
            }))
            .send()?
            .error_for_status()?;
        println!(
            "requested {reviewer} review for PR #{pr_number}: {}",
            response.status()
        );
        Ok(())
    }

    fn merge_pull_request(&self, repo: Option<&str>, pr_number: i64) -> anyhow::Result<()> {
        let git_api_url = self
            .git_api_url
            .as_ref()
            .ok_or_else(|| anyhow!("missing --git-api-url for merge request"))?;
        let repo = repo.ok_or_else(|| anyhow!("repo is required to merge pull request"))?;
        let (owner, name) = repo
            .split_once('/')
            .ok_or_else(|| anyhow!("repo must be formatted as owner/name"))?;
        let mut request = reqwest::blocking::Client::new()
            .patch(format!(
                "{}/api/repos/{owner}/{name}/pulls/{pr_number}",
                git_api_url.trim_end_matches('/')
            ))
            .json(&serde_json::json!({ "state": "merged" }));
        if let Some(token) = &self.git_token {
            request = request.bearer_auth(token);
        }
        let response = request.send()?.error_for_status()?;
        println!("merged {repo} PR #{pr_number}: {}", response.status());
        Ok(())
    }
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
            None,
            None,
            None,
        )
        .unwrap();
        assert!(command
            .bin
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("gild"));
        assert_eq!(
            command.args,
            [
                "chain-daemon",
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
                dispatcher_url: None,
                git_api_url: None,
                git_token: None,
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
        assert!(captured[0].args.contains(&state_db.display().to_string()));
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
        let chain = crate::chain_core::new_chain(
            Policy::DefaultFlow,
            "agent-khalid".into(),
            "do work".into(),
        );
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
