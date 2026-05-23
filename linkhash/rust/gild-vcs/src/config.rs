use std::path::PathBuf;

pub const SERVICE_NAME: &str = "linkhash";

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub repos_dir: String,
    pub db_path: String,
}

/// Read an env var, trying LINKHASH_ prefix first, then TANA_GIT_ for backwards compat.
fn env_or(linkhash_key: &str, tana_key: &str) -> Option<String> {
    std::env::var(linkhash_key)
        .ok()
        .or_else(|| std::env::var(tana_key).ok())
}

impl Config {
    pub fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());

        let repos_dir = env_or("LINKHASH_REPOS_DIR", "TANA_GIT_REPOS_DIR")
            .unwrap_or_else(|| format!("{}/Projects/tana/store/repos", home));

        let db_path = env_or("LINKHASH_DB_PATH", "TANA_GIT_DB_PATH")
            .unwrap_or_else(|| format!("{}/Projects/tana/git-server/data/tana-git.db", home));

        let port = env_or("LINKHASH_PORT", "TANA_GIT_PORT")
            .and_then(|v| v.parse().ok())
            .unwrap_or(9418);

        Config {
            port,
            repos_dir,
            db_path,
        }
    }

    pub fn db_dir(&self) -> PathBuf {
        PathBuf::from(&self.db_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf()
    }
}
