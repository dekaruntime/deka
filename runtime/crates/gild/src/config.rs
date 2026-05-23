use serde::Deserialize;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use crate::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub slug: String,
    pub port: Option<u16>,
    pub name: Option<String>,
    pub sandbox: Option<String>,
}

#[derive(Debug)]
pub struct AgentRegistry {
    pub agents: Vec<Agent>,
}

#[derive(Deserialize)]
struct AgentsToml {
    agents: BTreeMap<String, TomlAgent>,
}

#[derive(Deserialize)]
struct TomlAgent {
    port: Option<u16>,
    name: Option<String>,
    sandbox: Option<String>,
}

#[derive(Deserialize)]
struct AgentPortsJson {
    ports: BTreeMap<String, u16>,
    personas: Option<BTreeMap<String, PersonaJson>>,
}

#[derive(Deserialize)]
struct PersonaJson {
    name: Option<String>,
    sandbox: Option<String>,
}

impl AgentRegistry {
    pub fn load() -> Result<Self> {
        if let Some(path) = env_path("GILD_AGENTS_CONFIG") {
            return Self::from_agents_toml(&path);
        }
        if let Some(path) = find_config("infra/gild-config/agents.toml") {
            return Self::from_agents_toml(&path);
        }
        if let Some(path) = env_path("GILD_AGENT_PORTS_CONFIG") {
            return Self::from_agent_ports_json(&path);
        }
        if let Some(path) = find_config("infra/agent-ports.json") {
            return Self::from_agent_ports_json(&path);
        }
        Err("could not find infra/gild-config/agents.toml or infra/agent-ports.json".into())
    }

    pub fn find(&self, slug: &str) -> Option<&Agent> {
        self.agents
            .iter()
            .find(|agent| agent.slug == slug)
            .or_else(|| {
                self.agents
                    .iter()
                    .find(|agent| agent.slug == format!("agent-{slug}"))
            })
    }

    fn from_agents_toml(path: &Path) -> Result<Self> {
        let config: AgentsToml = toml::from_str(&fs::read_to_string(path)?)?;
        let mut agents = config
            .agents
            .into_iter()
            .map(|(slug, agent)| Agent {
                slug,
                port: agent.port,
                name: agent.name,
                sandbox: agent.sandbox,
            })
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| left.slug.cmp(&right.slug));
        Ok(Self { agents })
    }

    fn from_agent_ports_json(path: &Path) -> Result<Self> {
        let config: AgentPortsJson = serde_json::from_str(&fs::read_to_string(path)?)?;
        let mut agents = config
            .ports
            .into_iter()
            .map(|(slug, port)| {
                let persona = config
                    .personas
                    .as_ref()
                    .and_then(|personas| personas.get(&slug));
                Agent {
                    slug,
                    port: Some(port),
                    name: persona.and_then(|persona| persona.name.clone()),
                    sandbox: persona.and_then(|persona| persona.sandbox.clone()),
                }
            })
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| left.slug.cmp(&right.slug));
        Ok(Self { agents })
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.exists())
}

fn find_config(relative: &str) -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join(relative);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let home_candidate = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Projects/tana").join(relative));
    home_candidate.filter(|path| path.exists())
}

#[cfg(test)]
mod tests {
    use super::AgentRegistry;
    use std::{fs, time::SystemTime};

    #[test]
    fn reads_agents_toml() {
        let dir = std::env::temp_dir().join(format!(
            "gild-agents-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agents.toml");
        fs::write(
            &path,
            r#"
[agents.agent-khalid]
port = 9434
name = "Khalid"
sandbox = "gild"
"#,
        )
        .unwrap();

        let registry = AgentRegistry::from_agents_toml(&path).unwrap();
        assert_eq!(registry.find("khalid").unwrap().port, Some(9434));
    }
}
