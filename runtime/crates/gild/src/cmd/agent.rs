use clap::{Args, Subcommand};
use serde::Serialize;
use std::future::Future;
use std::io::{self, Write};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Command;
use std::sync::OnceLock;

use crate::{
    agent_client::{
        AgentClientResult, DeleteUnitResponse, GildAgentClient, SystemctlResponse, UserdelResponse,
    },
    config::AgentRegistry,
    Result,
};

const DEFAULT_GILD_AGENT_SOCKET: &str = "/run/gild-agent.sock";
const TANA_DIR: &str = "/home/sami/Projects/tana";
const BUN_BIN: &str = "/home/sami/.bun/bin/bun";
const GILD_SOCKET_PATH: &str = "/run/gild/sock";
const DISPATCHER_HMAC_KEY_PLACEHOLDER: &str = "__SET_BY_GILD_AGENT__";

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    command: AgentCommand,
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// List registered agents.
    Ls {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create a new agent through gild-agent.
    Create {
        slug: String,
        /// Print the planned privileged operations without contacting gild-agent.
        #[arg(long)]
        dry_run: bool,
    },
    /// Enable and start an agent dispatcher.
    Enable {
        slug: String,
        /// Enable the unit without starting it immediately.
        #[arg(long)]
        no_start: bool,
    },
    /// Stop and disable an agent dispatcher.
    Disable {
        slug: String,
        /// Disable the unit without stopping a running instance.
        #[arg(long)]
        no_stop: bool,
    },
    /// Delete an agent Linux user after disabling its dispatcher.
    Delete {
        slug: String,
        /// Confirm deletion without an interactive prompt.
        #[arg(long)]
        yes: bool,
    },
}

pub async fn run(args: AgentArgs) -> Result<()> {
    match args.command {
        AgentCommand::Ls { json } => list_agents(json),
        AgentCommand::Create { slug, dry_run } => create_agent(&slug, dry_run).await,
        AgentCommand::Enable { slug, no_start } => enable_agent(&slug, no_start).await,
        AgentCommand::Disable { slug, no_stop } => disable_agent(&slug, no_stop).await,
        AgentCommand::Delete { slug, yes } => delete_agent(&slug, yes).await,
    }
}

fn list_agents(json: bool) -> Result<()> {
    let registry = match AgentRegistry::load() {
        Ok(registry) => registry,
        Err(err) => {
            if json {
                println!("[]");
            } else {
                println!("no registry found: {err}");
            }
            return Ok(());
        }
    };
    let rows = registry
        .agents
        .into_iter()
        .map(|agent| {
            let active = systemd_active_state(&agent.slug);
            AgentListRow {
                slug: agent.slug,
                port: agent.port,
                sandbox: agent.sandbox.unwrap_or_else(|| "-".to_string()),
                persona: agent.name.unwrap_or_else(|| "-".to_string()),
                active,
            }
        })
        .collect::<Vec<_>>();

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "{:<18} {:<8} {:<12} {:<18} ACTIVE",
        "SLUG", "PORT", "SANDBOX", "PERSONA"
    );
    for row in rows {
        let port = row
            .port
            .map(|port| port.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<18} {:<8} {:<12} {:<18} {}",
            row.slug, port, row.sandbox, row.persona, row.active
        );
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct AgentListRow {
    slug: String,
    port: Option<u16>,
    sandbox: String,
    persona: String,
    active: String,
}

fn systemd_active_state(slug: &str) -> String {
    let primary = dispatcher_unit_name(slug);
    let legacy_slug = slug.strip_prefix("agent-").unwrap_or(slug);
    let legacy = format!("gg.tana.agent-{legacy_slug}.service");
    systemctl_is_active(&primary)
        .or_else(|| systemctl_is_active(&legacy))
        .unwrap_or_else(|| "inactive".to_string())
}

fn systemctl_is_active(unit: &str) -> Option<String> {
    let output = Command::new(systemctl_bin())
        .args(["is-active", unit])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Some(if stdout.is_empty() {
            "active".to_string()
        } else {
            stdout
        })
    } else {
        None
    }
}

fn systemctl_bin() -> String {
    std::env::var("GILD_SYSTEMCTL_BIN").unwrap_or_else(|_| "systemctl".to_string())
}

async fn create_agent(slug: &str, dry_run: bool) -> Result<()> {
    validate_agent_slug(slug)?;

    let unit = dispatcher_unit_name(slug);
    let port = dispatcher_port(slug)?;
    let unit_contents = dispatcher_unit_contents(slug, port);
    let socket_path = gild_agent_socket_path();

    if dry_run {
        println!("dry run: would create agent {slug}");
        println!("socket: {}", socket_path.display());
        println!("1. useradd {slug}");
        println!("2. hmac_rotate {slug}");
        println!("3. write_unit {unit}");
        println!("4. systemctl daemon-reload");
        println!("5. systemctl enable {unit}");
        println!();
        println!("{unit}:");
        print!("{unit_contents}");
        return Ok(());
    }

    let client = GildAgentClient::new(socket_path);
    client.useradd(slug).await?;
    let hmac = client.hmac_rotate(slug).await?;
    let write_unit = match client.write_unit(slug, port).await {
        Ok(response) => response,
        Err(err) if err.status() == Some(409) => {
            return Err(format!(
                "unit {unit} already exists with different contents; remove or reconcile the existing unit before retrying"
            )
            .into());
        }
        Err(err) => return Err(err.into()),
    };
    client.systemctl("daemon-reload", &unit).await?;
    client.systemctl("enable", &unit).await?;

    println!("created agent {slug}");
    println!("hmac fingerprint: {}", hmac.new_key_fingerprint);
    println!("unit: {}", write_unit.path);
    Ok(())
}

async fn enable_agent(slug: &str, no_start: bool) -> Result<()> {
    validate_agent_slug(slug)?;
    let client = GildAgentClient::new(gild_agent_socket_path());
    let summary = enable_agent_with_client(&client, slug, no_start).await?;
    println!("enabled {}", summary.unit);
    println!("active: {}", summary.active);
    Ok(())
}

async fn disable_agent(slug: &str, no_stop: bool) -> Result<()> {
    validate_agent_slug(slug)?;
    let client = GildAgentClient::new(gild_agent_socket_path());
    let unit = disable_agent_with_client(&client, slug, no_stop).await?;
    println!("disabled {unit}");
    Ok(())
}

async fn delete_agent(slug: &str, yes: bool) -> Result<()> {
    validate_agent_slug(slug)?;
    if !yes && !confirm_delete(slug)? {
        println!("delete cancelled");
        return Ok(());
    }

    let client = GildAgentClient::new(gild_agent_socket_path());
    let summary = delete_agent_with_client(&client, slug).await?;
    println!("deleted agent {slug}");
    for action in summary.actions {
        println!("- {action}");
    }
    Ok(())
}

fn confirm_delete(slug: &str) -> Result<bool> {
    print!("Delete {slug}? Type 'yes' to continue: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim() == "yes")
}

#[derive(Debug, PartialEq, Eq)]
struct EnableSummary {
    unit: String,
    active: String,
}

#[derive(Debug, PartialEq, Eq)]
struct DeleteSummary {
    actions: Vec<String>,
}

type BoxAgentFuture<'a, T> = Pin<Box<dyn Future<Output = AgentClientResult<T>> + Send + 'a>>;

trait AgentOps {
    fn userdel<'a>(&'a self, slug: &'a str) -> BoxAgentFuture<'a, UserdelResponse>;
    fn systemctl<'a>(
        &'a self,
        action: &'a str,
        unit: &'a str,
    ) -> BoxAgentFuture<'a, SystemctlResponse>;
    fn delete_unit<'a>(&'a self, unit: &'a str) -> BoxAgentFuture<'a, DeleteUnitResponse>;
}

impl AgentOps for GildAgentClient {
    fn userdel<'a>(&'a self, slug: &'a str) -> BoxAgentFuture<'a, UserdelResponse> {
        Box::pin(self.userdel(slug))
    }

    fn systemctl<'a>(
        &'a self,
        action: &'a str,
        unit: &'a str,
    ) -> BoxAgentFuture<'a, SystemctlResponse> {
        Box::pin(self.systemctl(action, unit))
    }

    fn delete_unit<'a>(&'a self, unit: &'a str) -> BoxAgentFuture<'a, DeleteUnitResponse> {
        Box::pin(self.delete_unit(unit))
    }
}

async fn enable_agent_with_client<C: AgentOps>(
    client: &C,
    slug: &str,
    no_start: bool,
) -> Result<EnableSummary> {
    let unit = dispatcher_unit_name(slug);
    client.systemctl("enable", &unit).await?;
    if !no_start {
        client.systemctl("start", &unit).await?;
    }
    let active = match client.systemctl("status", &unit).await {
        Ok(response) => parse_active_state(&response.stdout),
        Err(_) if no_start => "unknown".to_string(),
        Err(err) => return Err(err.into()),
    };
    Ok(EnableSummary { unit, active })
}

async fn disable_agent_with_client<C: AgentOps>(
    client: &C,
    slug: &str,
    no_stop: bool,
) -> Result<String> {
    let unit = dispatcher_unit_name(slug);
    if !no_stop {
        client.systemctl("stop", &unit).await?;
    }
    client.systemctl("disable", &unit).await?;
    Ok(unit)
}

async fn delete_agent_with_client<C: AgentOps>(client: &C, slug: &str) -> Result<DeleteSummary> {
    let unit = dispatcher_unit_name(slug);
    let mut actions = Vec::new();
    match client.systemctl("stop", &unit).await {
        Ok(_) => actions.push(format!("stopped {unit}")),
        Err(err) => actions.push(format!("stop best-effort failed: {err}")),
    }
    match client.systemctl("disable", &unit).await {
        Ok(_) => actions.push(format!("disabled {unit}")),
        Err(err) => actions.push(format!("disable best-effort failed: {err}")),
    }
    client.userdel(slug).await?;
    actions.push(format!("removed Linux user {slug}"));
    match client.delete_unit(&unit).await {
        Ok(response) if response.existed => {
            actions.push(format!("deleted unit {}", response.path));
        }
        Ok(response) => {
            actions.push(format!("unit already absent {}", response.path));
        }
        Err(err) => actions.push(format!("delete_unit best-effort failed: {err}")),
    }
    Ok(DeleteSummary { actions })
}

fn parse_active_state(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("Active:"))
        .and_then(|active| active.split_whitespace().next())
        .unwrap_or("unknown")
        .to_string()
}

pub(crate) fn validate_agent_slug(slug: &str) -> Result<()> {
    if slug_regex().is_match(slug) {
        Ok(())
    } else {
        Err("slug must match ^agent-[a-z][a-z0-9-]{1,30}$".into())
    }
}

fn slug_regex() -> &'static regex::Regex {
    static SLUG_RE: OnceLock<regex::Regex> = OnceLock::new();
    SLUG_RE.get_or_init(|| {
        regex::Regex::new(r"^agent-[a-z][a-z0-9-]{1,30}$").expect("agent slug regex compiles")
    })
}

fn gild_agent_socket_path() -> PathBuf {
    std::env::var("GILD_AGENT_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_GILD_AGENT_SOCKET))
}

fn dispatcher_unit_name(slug: &str) -> String {
    format!("gg.tana.gild-dispatcher@{slug}.service")
}

fn dispatcher_port(slug: &str) -> Result<u16> {
    AgentRegistry::load()?
        .find(slug)
        .and_then(|agent| agent.port)
        .ok_or_else(|| format!("agent {slug} has no dispatcher port in the registry").into())
}

fn dispatcher_unit_contents(slug: &str, port: u16) -> String {
    format!(
        r#"[Unit]
Description=Tana gild dispatcher for {slug}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={slug}
Group={slug}
WorkingDirectory=/home/{slug}

Environment="PATH=/home/{slug}/.local/bin:/home/{slug}/.bun/bin:/usr/local/bin:/usr/bin:/bin"
Environment="HOME=/home/{slug}"
Environment="AGENT_SLUG={slug}"
Environment="AGENT_DISPATCHER_PORT={port}"
Environment="AGENT_WORKSPACE=/home/{slug}/repos"
Environment="AGENT_DISPATCHER_HMAC_KEY={DISPATCHER_HMAC_KEY_PLACEHOLDER}"
Environment="AGENT_PORTS_FILE={TANA_DIR}/infra/agent-ports.json"
Environment="AGENT_WORKER_SANDBOX=host"
Environment="GILD_SOCKET_PATH={GILD_SOCKET_PATH}"
Environment="GILD_BEARER_TOKEN_FILE=/home/{slug}/.config/tana/gild-bearer-token"

ExecStart={BUN_BIN} run {TANA_DIR}/agent-dispatcher/src/index.ts
ExecStartPost=/bin/sh -c 'for attempt in 1 2 3 4 5 6 7 8 9 10; do /usr/bin/curl -fsS --max-time 2 http://127.0.0.1:${{AGENT_DISPATCHER_PORT}}/health && exit 0; sleep 1; done; exit 1'

Restart=always
RestartSec=3
StandardOutput=append:/var/log/gg.tana.gild-dispatcher-{slug}.log
StandardError=append:/var/log/gg.tana.gild-dispatcher-{slug}.log

NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/home/{slug} /var/log /tmp
PrivateTmp=true

[Install]
WantedBy=multi-user.target
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn slug_validator_accepts_agent_slugs() {
        for slug in [
            "agent-ab",
            "agent-a1",
            "agent-a-b-c",
            "agent-a234567890123456789012345678901",
        ] {
            validate_agent_slug(slug).unwrap();
        }
    }

    #[test]
    fn slug_validator_rejects_invalid_slugs() {
        for slug in [
            "agent-a",
            "Agent-ab",
            "agent-1a",
            "agent-a_b",
            "agent-a.b",
            "other-agent-ab",
            "agent-a2345678901234567890123456789012",
        ] {
            assert!(validate_agent_slug(slug).is_err(), "{slug}");
        }
    }

    #[test]
    fn dispatcher_unit_substitutes_slug() {
        let unit = dispatcher_unit_contents("agent-zed", 9430);
        assert_eq!(
            unit,
            r#"[Unit]
Description=Tana gild dispatcher for agent-zed
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=agent-zed
Group=agent-zed
WorkingDirectory=/home/agent-zed

Environment="PATH=/home/agent-zed/.local/bin:/home/agent-zed/.bun/bin:/usr/local/bin:/usr/bin:/bin"
Environment="HOME=/home/agent-zed"
Environment="AGENT_SLUG=agent-zed"
Environment="AGENT_DISPATCHER_PORT=9430"
Environment="AGENT_WORKSPACE=/home/agent-zed/repos"
Environment="AGENT_DISPATCHER_HMAC_KEY=__SET_BY_GILD_AGENT__"
Environment="AGENT_PORTS_FILE=/home/sami/Projects/tana/infra/agent-ports.json"
Environment="AGENT_WORKER_SANDBOX=host"
Environment="GILD_SOCKET_PATH=/run/gild/sock"
Environment="GILD_BEARER_TOKEN_FILE=/home/agent-zed/.config/tana/gild-bearer-token"

ExecStart=/home/sami/.bun/bin/bun run /home/sami/Projects/tana/agent-dispatcher/src/index.ts
ExecStartPost=/bin/sh -c 'for attempt in 1 2 3 4 5 6 7 8 9 10; do /usr/bin/curl -fsS --max-time 2 http://127.0.0.1:${AGENT_DISPATCHER_PORT}/health && exit 0; sleep 1; done; exit 1'

Restart=always
RestartSec=3
StandardOutput=append:/var/log/gg.tana.gild-dispatcher-agent-zed.log
StandardError=append:/var/log/gg.tana.gild-dispatcher-agent-zed.log

NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/home/agent-zed /var/log /tmp
PrivateTmp=true

[Install]
WantedBy=multi-user.target
"#
        );
    }

    #[test]
    fn parses_active_state_from_systemctl_status() {
        assert_eq!(
            parse_active_state(
                "● unit.service\n     Loaded: loaded\n     Active: active (running) since now\n"
            ),
            "active"
        );
        assert_eq!(parse_active_state("no active line"), "unknown");
    }

    #[tokio::test]
    async fn enable_sequence_starts_and_reads_status() {
        let client = MockAgentOps::default();
        let summary = enable_agent_with_client(&client, "agent-zed", false)
            .await
            .unwrap();

        assert_eq!(
            client.calls(),
            vec![
                "systemctl enable gg.tana.gild-dispatcher@agent-zed.service",
                "systemctl start gg.tana.gild-dispatcher@agent-zed.service",
                "systemctl status gg.tana.gild-dispatcher@agent-zed.service",
            ]
        );
        assert_eq!(summary.active, "active");
    }

    #[tokio::test]
    async fn enable_no_start_skips_start() {
        let client = MockAgentOps::default();
        enable_agent_with_client(&client, "agent-zed", true)
            .await
            .unwrap();

        assert_eq!(
            client.calls(),
            vec![
                "systemctl enable gg.tana.gild-dispatcher@agent-zed.service",
                "systemctl status gg.tana.gild-dispatcher@agent-zed.service",
            ]
        );
    }

    #[tokio::test]
    async fn disable_sequence_stops_then_disables() {
        let client = MockAgentOps::default();
        let unit = disable_agent_with_client(&client, "agent-zed", false)
            .await
            .unwrap();

        assert_eq!(unit, "gg.tana.gild-dispatcher@agent-zed.service");
        assert_eq!(
            client.calls(),
            vec![
                "systemctl stop gg.tana.gild-dispatcher@agent-zed.service",
                "systemctl disable gg.tana.gild-dispatcher@agent-zed.service",
            ]
        );
    }

    #[tokio::test]
    async fn disable_no_stop_skips_stop() {
        let client = MockAgentOps::default();
        disable_agent_with_client(&client, "agent-zed", true)
            .await
            .unwrap();

        assert_eq!(
            client.calls(),
            vec!["systemctl disable gg.tana.gild-dispatcher@agent-zed.service"]
        );
    }

    #[tokio::test]
    async fn delete_sequence_best_effort_systemctl_then_userdel() {
        let client = MockAgentOps::default();
        let summary = delete_agent_with_client(&client, "agent-zed")
            .await
            .unwrap();

        assert_eq!(
            client.calls(),
            vec![
                "systemctl stop gg.tana.gild-dispatcher@agent-zed.service",
                "systemctl disable gg.tana.gild-dispatcher@agent-zed.service",
                "userdel agent-zed",
                "delete_unit gg.tana.gild-dispatcher@agent-zed.service",
            ]
        );
        assert!(summary.actions.iter().any(|action| action
            == "deleted unit /etc/systemd/system/gg.tana.gild-dispatcher@agent-zed.service"));
    }

    #[derive(Default)]
    struct MockAgentOps {
        calls: Mutex<Vec<String>>,
    }

    impl MockAgentOps {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl AgentOps for MockAgentOps {
        fn userdel<'a>(&'a self, slug: &'a str) -> BoxAgentFuture<'a, UserdelResponse> {
            self.calls.lock().unwrap().push(format!("userdel {slug}"));
            Box::pin(async {
                Ok(UserdelResponse {
                    ok: true,
                    operation: "agent.remove".to_string(),
                    message: "removed".to_string(),
                    exit: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                    error: None,
                    pending: Vec::new(),
                })
            })
        }

        fn systemctl<'a>(
            &'a self,
            action: &'a str,
            unit: &'a str,
        ) -> BoxAgentFuture<'a, SystemctlResponse> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("systemctl {action} {unit}"));
            Box::pin(async move {
                Ok(SystemctlResponse {
                    ok: true,
                    op: "systemctl".to_string(),
                    action: action.to_string(),
                    unit: unit.to_string(),
                    exit: Some(0),
                    stdout: if action == "status" {
                        "Active: active (running)".to_string()
                    } else {
                        String::new()
                    },
                    stderr: String::new(),
                    error: None,
                    audit_error: None,
                })
            })
        }

        fn delete_unit<'a>(&'a self, unit: &'a str) -> BoxAgentFuture<'a, DeleteUnitResponse> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("delete_unit {unit}"));
            Box::pin(async move {
                Ok(DeleteUnitResponse {
                    ok: true,
                    path: format!("/etc/systemd/system/{unit}"),
                    existed: true,
                })
            })
        }
    }
}
