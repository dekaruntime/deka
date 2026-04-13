use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

pub const SERVICE_NAME: &str = "deka-git";

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_database_url")]
    pub database_url: String,

    #[serde(default = "default_repos_path")]
    pub repos_path: String,

    #[serde(default = "default_bootstrap_username")]
    pub bootstrap_username: String,

    #[serde(default = "default_bootstrap_token")]
    pub bootstrap_token: String,

    #[serde(default = "default_auto_verify_signups")]
    pub auto_verify_signups: bool,
}

fn default_port() -> u16 {
    8508
}

fn default_database_url() -> String {
    "postgres://deka:deka_dev_password@localhost:5434/deka".to_string()
}

fn default_repos_path() -> String {
    "./repos".to_string()
}

fn default_bootstrap_username() -> String {
    "linkhash-admin".to_string()
}

fn default_bootstrap_token() -> String {
    "linkhash-dev-token-change-me".to_string()
}

fn default_auto_verify_signups() -> bool {
    true
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let paths = config_search_paths();
        for path in paths {
            if path.exists() {
                let content = fs::read_to_string(&path)?;
                let cfg: Config = toml::from_str(&content)?;
                tracing::info!("Loaded config from {}", path.display());
                return Ok(cfg);
            }
        }

        tracing::warn!(
            "No config.toml found for {}; using built-in defaults",
            SERVICE_NAME
        );
        Ok(Config {
            port: default_port(),
            database_url: default_database_url(),
            repos_path: default_repos_path(),
            bootstrap_username: default_bootstrap_username(),
            bootstrap_token: default_bootstrap_token(),
            auto_verify_signups: default_auto_verify_signups(),
        })
    }
}

fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("./config.toml")];

    if let Ok(home) = std::env::var("HOME") {
        paths.push(
            PathBuf::from(home)
                .join(".config")
                .join(SERVICE_NAME)
                .join("config.toml"),
        );
    }

    paths.push(PathBuf::from(format!("/etc/{}/config.toml", SERVICE_NAME)));
    paths
}
