//! Shard config loading.
//!
//! The config is a simple JSON document listing each shard's
//! connection endpoints. It is read once at startup and is
//! intended to be identical on every server in the cluster.

use std::path::Path;

/// One shard entry in the cluster.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ShardInfo {
    pub index: usize,
    pub name: String,
    pub neo4j: String,
    pub redis: String,
}

/// The cluster shard config.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ShardConfig {
    pub shards: Vec<ShardInfo>,
}

impl ShardConfig {
    /// Load a shard config from a JSON file on disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read shard config {}: {e}", path.display()))?;
        let cfg: ShardConfig = serde_json::from_str(&raw)
            .map_err(|e| format!("failed to parse shard config {}: {e}", path.display()))?;
        Ok(cfg)
    }

    /// A trivial single-shard config pointing at localhost. Used as
    /// the dev/default fallback when no config file is present.
    pub fn single_shard_localhost() -> Self {
        ShardConfig {
            shards: vec![ShardInfo {
                index: 0,
                name: "local".to_string(),
                neo4j: "bolt://127.0.0.1:7687".to_string(),
                redis: "redis://127.0.0.1:6379".to_string(),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn roundtrip_json() {
        let cfg = ShardConfig {
            shards: vec![
                ShardInfo {
                    index: 0,
                    name: "phobos".into(),
                    neo4j: "bolt://100.113.2.21:7687".into(),
                    redis: "redis://100.113.2.21:6379".into(),
                },
                ShardInfo {
                    index: 1,
                    name: "bugsy".into(),
                    neo4j: "bolt://100.70.138.96:7687".into(),
                    redis: "redis://100.70.138.96:6379".into(),
                },
            ],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ShardConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn from_file_ok() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("shards-{}.json", std::process::id()));
        let body = r#"{
            "shards": [
                { "index": 0, "name": "a", "neo4j": "bolt://x:7687", "redis": "redis://x:6379" }
            ]
        }"#;
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(body.as_bytes()).unwrap();
        }
        let cfg = ShardConfig::from_file(&path).unwrap();
        assert_eq!(cfg.shards.len(), 1);
        assert_eq!(cfg.shards[0].name, "a");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_file_missing_is_error() {
        let err = ShardConfig::from_file("/nonexistent/path/shards.json");
        assert!(err.is_err());
    }

    #[test]
    fn single_shard_localhost_is_valid() {
        let cfg = ShardConfig::single_shard_localhost();
        assert_eq!(cfg.shards.len(), 1);
        assert_eq!(cfg.shards[0].index, 0);
    }
}
