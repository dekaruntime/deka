use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::{agent_client::GildAgentClient, config::AgentRegistry, Result};

const DEFAULT_GILD_AGENT_SOCKET: &str = "/run/gild-agent.sock";
const DISPATCHER_BINARY: &str = "/usr/local/bin/gild-dispatcher";
const DISPATCHER_GROUP: &str = "gild-orchestrator";

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    command: AgentCommand,
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// List registered agents.
    Ls,
    /// Create a new agent through gild-agent.
    Create {
        slug: String,
        /// Print the planned privileged operations without contacting gild-agent.
        #[arg(long)]
        dry_run: bool,
    },
    /// Enable an agent sandbox mode.
    Enable {
        slug: String,
        #[arg(long)]
        sandbox: SandboxMode,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum SandboxMode {
    Vm,
    Host,
}

pub async fn run(args: AgentArgs) -> Result<()> {
    match args.command {
        AgentCommand::Ls => list_agents(),
        AgentCommand::Create { slug, dry_run } => create_agent(&slug, dry_run).await,
        AgentCommand::Enable { .. } => {
            println!("not yet implemented in this skeleton PR");
            Ok(())
        }
    }
}

fn list_agents() -> Result<()> {
    let registry = AgentRegistry::load()?;
    println!("{:<18} {:<8} {:<10} NAME", "SLUG", "PORT", "SANDBOX");
    for agent in registry.agents {
        let name = agent.name.unwrap_or_else(|| "-".to_string());
        let sandbox = agent.sandbox.unwrap_or_else(|| "-".to_string());
        let port = agent
            .port
            .map(|port| port.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!("{:<18} {:<8} {:<10} {}", agent.slug, port, sandbox, name);
    }
    Ok(())
}

async fn create_agent(slug: &str, dry_run: bool) -> Result<()> {
    validate_agent_slug(slug)?;

    let unit = dispatcher_unit_name(slug);
    let unit_contents = dispatcher_unit_contents(slug);
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
    let write_unit = match client.write_unit(&unit, &unit_contents).await {
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

fn dispatcher_unit_contents(slug: &str) -> String {
    format!(
        r#"[Unit]
Description=Tana gild dispatcher for {slug}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={slug}
Group={DISPATCHER_GROUP}
WorkingDirectory=/home/{slug}
Environment=AGENT_SLUG={slug}
ExecStart={DISPATCHER_BINARY}
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let unit = dispatcher_unit_contents("agent-zed");
        assert!(unit.contains("User=agent-zed"));
        assert!(unit.contains("Group=gild-orchestrator"));
        assert!(unit.contains("WorkingDirectory=/home/agent-zed"));
        assert!(unit.contains("Environment=AGENT_SLUG=agent-zed"));
        assert!(unit.contains(DISPATCHER_BINARY));
    }
}
