use anyhow::Context;
use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Debug, Deserialize)]
pub struct InstallPayload {
    #[serde(default)]
    pub specs: Vec<String>,
    #[serde(default)]
    pub yes: bool,
    #[serde(default)]
    pub prompt: bool,
    #[serde(default)]
    pub quiet: bool,
    #[serde(default)]
    pub rehash: bool,
}

impl InstallPayload {
    pub fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read payload from {}", path.display()))?;
        let payload: InstallPayload =
            serde_json::from_str(&contents).context("failed to parse install payload JSON")?;
        Ok(payload)
    }

    pub fn from_parts(specs: Vec<String>) -> Self {
        Self {
            specs,
            yes: false,
            prompt: false,
            quiet: false,
            rehash: false,
        }
    }
}
