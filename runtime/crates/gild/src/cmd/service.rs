use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use gild_vault_client::VaultClient;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::Result;

#[derive(Debug, Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Link provider credentials to an agent.
    #[command(group(
        ArgGroup::new("secret_source")
            .required(true)
            .multiple(false)
            .args(["from", "token"])
    ))]
    Link {
        /// Agent slug, for example agent-khalid.
        #[arg(long)]
        agent: String,
        /// Provider to link.
        #[arg(long)]
        provider: Provider,
        /// Read provider auth JSON from this file.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Store this token directly.
        #[arg(long)]
        token: Option<String>,
    },
    /// List linked provider credentials.
    Ls {
        /// Filter to one agent slug.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Remove linked provider credentials for an agent.
    Unlink {
        /// Agent slug, for example agent-khalid.
        #[arg(long)]
        agent: String,
        /// Provider to unlink.
        #[arg(long)]
        provider: Provider,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Provider {
    Codex,
    Opencode,
    Claude,
    Cloudflare,
    Anthropic,
}

pub async fn run(args: ServiceArgs) -> Result<()> {
    let client = VaultClient::from_socket();
    match args.command {
        ServiceCommand::Link {
            agent,
            provider,
            from,
            token,
        } => link(&client, &agent, provider, from, token).await,
        ServiceCommand::Ls { agent } => list(&client, agent.as_deref()).await,
        ServiceCommand::Unlink { agent, provider } => unlink(&client, &agent, provider).await,
    }
}

async fn link(
    client: &VaultClient,
    agent: &str,
    provider: Provider,
    from: Option<PathBuf>,
    token: Option<String>,
) -> Result<()> {
    validate_agent_slug(agent)?;
    let (kind, value) = match (from, token) {
        (Some(path), None) => ("AUTH", std::fs::read_to_string(&path)?),
        (None, Some(token)) => ("TOKEN", token),
        _ => return Err("exactly one of --from or --token is required".into()),
    };
    let key = vault_key(agent, provider, kind);
    client.put(&key, &value).await?;
    let digest = sha256_prefix(&value);
    println!(
        "linked {} {} for {} (vault key {}, sha256 {}...)",
        provider.as_str(),
        kind.to_ascii_lowercase(),
        agent,
        key,
        digest
    );
    Ok(())
}

async fn list(client: &VaultClient, agent: Option<&str>) -> Result<()> {
    if let Some(agent) = agent {
        validate_agent_slug(agent)?;
    }

    let keys = client.list().await?;
    let rows = keys
        .into_iter()
        .filter_map(|key| parse_service_key(&key))
        .filter(|row| agent.is_none_or(|agent| row.agent == agent))
        .collect::<Vec<_>>();

    if rows.is_empty() {
        println!("no linked service credentials");
        return Ok(());
    }

    let mut by_agent = BTreeMap::<String, Vec<ServiceLinkRow>>::new();
    for row in rows {
        by_agent.entry(row.agent.clone()).or_default().push(row);
    }

    for (agent, mut rows) in by_agent {
        rows.sort_by(|left, right| {
            left.provider
                .cmp(&right.provider)
                .then(left.kind.cmp(&right.kind))
        });
        for row in rows {
            println!("{} {:<10} {:<5} {}", agent, row.provider, row.kind, row.key);
        }
    }
    Ok(())
}

async fn unlink(client: &VaultClient, agent: &str, provider: Provider) -> Result<()> {
    validate_agent_slug(agent)?;
    let prefix = vault_key_prefix(agent, provider);
    let keys = client.list().await?;
    let matching = keys
        .into_iter()
        .filter(|key| key == &format!("{prefix}_AUTH") || key == &format!("{prefix}_TOKEN"))
        .collect::<Vec<_>>();

    if matching.is_empty() {
        println!("no linked {} credentials for {}", provider.as_str(), agent);
        return Ok(());
    }

    for key in &matching {
        client.delete(key).await?;
    }
    println!(
        "unlinked {} for {} (removed {})",
        provider.as_str(),
        agent,
        matching.join(", ")
    );
    Ok(())
}

fn validate_agent_slug(slug: &str) -> Result<()> {
    static AGENT_SLUG: OnceLock<Regex> = OnceLock::new();
    let regex = AGENT_SLUG.get_or_init(|| Regex::new(r"^agent-[a-z][a-z0-9-]{1,30}$").unwrap());
    if regex.is_match(slug) {
        Ok(())
    } else {
        Err(format!("agent slug must match ^agent-[a-z][a-z0-9-]{{1,30}}$: {slug}").into())
    }
}

fn vault_key(agent: &str, provider: Provider, kind: &str) -> String {
    format!("{}_{}", vault_key_prefix(agent, provider), kind)
}

fn vault_key_prefix(agent: &str, provider: Provider) -> String {
    format!("{}_{}", agent_env_prefix(agent), provider.env_name())
}

fn agent_env_prefix(agent: &str) -> String {
    agent.to_ascii_uppercase().replace('-', "_")
}

fn sha256_prefix(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex_prefix(&digest, 12)
}

fn hex_prefix(bytes: &[u8], len: usize) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(len);
    for byte in bytes {
        if encoded.len() >= len {
            break;
        }
        encoded.push(TABLE[(byte >> 4) as usize] as char);
        if encoded.len() >= len {
            break;
        }
        encoded.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn parse_service_key(key: &str) -> Option<ServiceLinkRow> {
    let (without_kind, kind) = key.rsplit_once('_')?;
    if kind != "AUTH" && kind != "TOKEN" {
        return None;
    }

    for provider in Provider::all() {
        let provider_suffix = format!("_{}", provider.env_name());
        let Some(agent_part) = without_kind.strip_suffix(&provider_suffix) else {
            continue;
        };
        if !agent_part.starts_with("AGENT_") {
            return None;
        }
        let agent = agent_part.to_ascii_lowercase().replace('_', "-");
        if validate_agent_slug(&agent).is_err() {
            return None;
        }
        return Some(ServiceLinkRow {
            agent,
            provider: provider.as_str().to_string(),
            kind: kind.to_ascii_lowercase(),
            key: key.to_string(),
        });
    }

    None
}

#[derive(Debug, PartialEq, Eq)]
struct ServiceLinkRow {
    agent: String,
    provider: String,
    kind: String,
    key: String,
}

impl Provider {
    const fn all() -> [Provider; 5] {
        [
            Provider::Codex,
            Provider::Opencode,
            Provider::Claude,
            Provider::Cloudflare,
            Provider::Anthropic,
        ]
    }

    const fn as_str(self) -> &'static str {
        match self {
            Provider::Codex => "codex",
            Provider::Opencode => "opencode",
            Provider::Claude => "claude",
            Provider::Cloudflare => "cloudflare",
            Provider::Anthropic => "anthropic",
        }
    }

    const fn env_name(self) -> &'static str {
        match self {
            Provider::Codex => "CODEX",
            Provider::Opencode => "OPENCODE",
            Provider::Claude => "CLAUDE",
            Provider::Cloudflare => "CLOUDFLARE",
            Provider::Anthropic => "ANTHROPIC",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_service_key, sha256_prefix, validate_agent_slug, vault_key, Provider, ServiceLinkRow,
    };

    #[test]
    fn validates_agent_slug() {
        assert!(validate_agent_slug("agent-amina").is_ok());
        assert!(validate_agent_slug("agent-a1-b2").is_ok());
        assert!(validate_agent_slug("amina").is_err());
        assert!(validate_agent_slug("agent-1bad").is_err());
        assert!(validate_agent_slug("agent-Bad").is_err());
    }

    #[test]
    fn derives_vault_keys() {
        assert_eq!(
            vault_key("agent-khalid", Provider::Codex, "AUTH"),
            "AGENT_KHALID_CODEX_AUTH"
        );
        assert_eq!(
            vault_key("agent-layla", Provider::Anthropic, "TOKEN"),
            "AGENT_LAYLA_ANTHROPIC_TOKEN"
        );
    }

    #[test]
    fn parses_service_keys() {
        assert_eq!(
            parse_service_key("AGENT_AMINA_CODEX_AUTH"),
            Some(ServiceLinkRow {
                agent: "agent-amina".to_string(),
                provider: "codex".to_string(),
                kind: "auth".to_string(),
                key: "AGENT_AMINA_CODEX_AUTH".to_string(),
            })
        );
        assert_eq!(parse_service_key("OTHER_CODEX_AUTH"), None);
        assert_eq!(parse_service_key("AGENT_AMINA_UNKNOWN_AUTH"), None);
    }

    #[test]
    fn hashes_value_for_confirmation() {
        assert_eq!(sha256_prefix("abc"), "ba7816bf8f01");
    }
}
