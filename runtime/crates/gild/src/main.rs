use clap::{Parser, Subcommand};

pub mod agent_client;
mod cmd;
mod config;
mod hmac;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Parser)]
#[command(name = "gild", version, about = "Tana agent orchestration CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Dispatch a one-off task to an agent.
    Dispatch(cmd::dispatch::DispatchArgs),
    /// Run chain orchestration policies.
    Chain(cmd::chain::ChainArgs),
    /// Inspect and manage registered agents.
    Agent(cmd::agent::AgentArgs),
    /// Read values from the gild vault.
    Vault(cmd::vault::VaultArgs),
    /// Inspect the gild VM pool.
    Pool(cmd::pool::PoolArgs),
    /// Link service credentials for an agent.
    Service(cmd::service::ServiceArgs),
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    match Cli::parse().command {
        Commands::Dispatch(args) => cmd::dispatch::run(args).await,
        Commands::Chain(args) => cmd::chain::run(args).await,
        Commands::Agent(args) => cmd::agent::run(args).await,
        Commands::Vault(args) => cmd::vault::run(args).await,
        Commands::Pool(args) => cmd::pool::run(args).await,
        Commands::Service(args) => cmd::service::run(args).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn parses_dispatch() {
        let cli = Cli::try_parse_from([
            "gild",
            "dispatch",
            "agent-khalid",
            "fix it",
            "--runtime",
            "codex",
            "--time-budget",
            "300",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Dispatch(_)));
    }

    #[test]
    fn parses_chain_run() {
        let cli =
            Cli::try_parse_from(["gild", "chain", "run", "--policy", "default-flow"]).unwrap();
        assert!(matches!(cli.command, Commands::Chain(_)));
    }

    #[test]
    fn parses_agent_commands() {
        for args in [
            vec!["gild", "agent", "ls"],
            vec!["gild", "agent", "create", "agent-zed"],
            vec!["gild", "agent", "enable", "agent-zed", "--sandbox", "vm"],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            assert!(matches!(cli.command, Commands::Agent(_)));
        }
    }

    #[test]
    fn parses_vault_pool_and_service() {
        for args in [
            vec!["gild", "vault", "get", "OPENAI_API_KEY"],
            vec!["gild", "pool", "status"],
            vec![
                "gild",
                "service",
                "link",
                "--agent",
                "agent-khalid",
                "--provider",
                "opencode",
                "--from",
                "/tmp/auth.json",
            ],
        ] {
            Cli::try_parse_from(args).unwrap();
        }
    }
}
