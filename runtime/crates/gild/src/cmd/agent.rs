use clap::{Args, Subcommand, ValueEnum};

use crate::{config::AgentRegistry, Result};

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
    Create { slug: String },
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
        AgentCommand::Create { slug } => {
            println!("would create agent {slug} via gild-agent socket");
            Ok(())
        }
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
