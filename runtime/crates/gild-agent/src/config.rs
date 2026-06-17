use anyhow::{Result, anyhow};
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use crate::auth::{group_gid, warn_agent_members_in_orchestrator};

pub(crate) const DEFAULT_SOCKET_PATH: &str = "/run/gild-agent.sock";
pub(crate) const ORCHESTRATOR_GROUP: &str = "gild-orchestrator";
pub(crate) const AGENT_RUNTIME_GROUP: &str = "gild-agents";
pub(crate) const TANA_DIR: &str = "/home/sami/Projects/tana";
pub(crate) const BUN_BIN: &str = "/home/sami/.bun/bin/bun";
pub(crate) const GILD_SOCKET_PATH: &str = "/run/gild/sock";
pub(crate) const DISPATCHER_HMAC_KEY_PLACEHOLDER: &str = "__SET_BY_GILD_AGENT__";
pub(crate) const READ_TIMEOUT_MS: u64 = 1_000;
pub(crate) const MAX_HEADER_BYTES: usize = 16 * 1024;
pub(crate) const MAX_BODY_BYTES: usize = 64 * 1024;
pub(crate) const DEFAULT_AUDIT_LOG_PATH: &str = "/var/log/gild-agent.log";
pub(crate) const PASSWD_PATH: &str = "/etc/passwd";
pub(crate) const GILD_AGENT_ROOT: &str = "/etc/gild/agents";
pub(crate) const DEFAULT_SYSTEMD_UNIT_ROOT: &str = "/etc/systemd/system";

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) started_at: Instant,
    pub(crate) socket_path: PathBuf,
    pub(crate) orchestrator_gid: u32,
    pub(crate) audit_log_path: PathBuf,
    pub(crate) passwd_path: PathBuf,
    pub(crate) systemd_unit_root: PathBuf,
    pub(crate) systemd_unit_owner: Option<(u32, u32)>,
    pub(crate) command_runner: Arc<dyn CommandRunner>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) exit: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) trait CommandRunner: Send + Sync {
    fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput>;
}

pub(crate) struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
        let output = Command::new(program)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            exit: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

pub(crate) fn load_state() -> Result<AppState> {
    let group = std::env::var("GILD_AGENT_GROUP").unwrap_or_else(|_| ORCHESTRATOR_GROUP.into());
    let orchestrator_gid =
        group_gid(&group)?.ok_or_else(|| anyhow!("group {group:?} not found"))?;
    warn_agent_members_in_orchestrator(PASSWD_PATH, "/etc/group", &group, orchestrator_gid);
    Ok(AppState {
        started_at: Instant::now(),
        socket_path: std::env::var("GILD_AGENT_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_SOCKET_PATH)),
        orchestrator_gid,
        audit_log_path: std::env::var("GILD_AGENT_AUDIT_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_AUDIT_LOG_PATH)),
        passwd_path: PathBuf::from(PASSWD_PATH),
        systemd_unit_root: std::env::var("GILD_AGENT_SYSTEMD_UNIT_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_SYSTEMD_UNIT_ROOT)),
        systemd_unit_owner: Some((0, 0)),
        command_runner: Arc::new(SystemCommandRunner),
    })
}
