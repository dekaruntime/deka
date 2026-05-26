use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PublisherState {
    pub last_seen_timestamp: i64,
}

pub fn state_file(state_dir: impl AsRef<Path>) -> PathBuf {
    state_dir.as_ref().join("state.json")
}

pub fn load_state(state_dir: impl AsRef<Path>) -> Result<PublisherState> {
    let path = state_file(state_dir);
    if !path.exists() {
        return Ok(PublisherState::default());
    }

    let data = fs::read_to_string(&path)
        .with_context(|| format!("failed to read state file {}", path.display()))?;
    serde_json::from_str(&data)
        .with_context(|| format!("failed to parse state file {}", path.display()))
}

pub fn save_state(state_dir: impl AsRef<Path>, state: PublisherState) -> Result<()> {
    let dir = state_dir.as_ref();
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create state directory {}", dir.display()))?;

    let path = state_file(dir);
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(&state).context("failed to serialize publisher state")?;
    fs::write(&tmp, data).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| {
        format!(
            "failed to replace state file {} with {}",
            path.display(),
            tmp.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_state_defaults_to_zero() {
        let dir = tempfile::tempdir().unwrap();
        let state = load_state(dir.path()).unwrap();
        assert_eq!(state.last_seen_timestamp, 0);
    }

    #[test]
    fn state_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        save_state(
            dir.path(),
            PublisherState {
                last_seen_timestamp: 12345,
            },
        )
        .unwrap();

        let state = load_state(dir.path()).unwrap();
        assert_eq!(state.last_seen_timestamp, 12345);
    }
}
