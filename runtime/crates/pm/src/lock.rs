use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const LOCKFILE_NAME: &str = "deka.lock";

#[derive(Debug, Serialize, Deserialize)]
pub struct EcosystemSection {
    pub packages: BTreeMap<String, LockEntry>,
}

impl Default for EcosystemSection {
    fn default() -> Self {
        Self {
            packages: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DekaLock {
    #[serde(rename = "lockfileVersion")]
    pub lockfile_version: u32,
    pub node: EcosystemSection,
    pub php: EcosystemSection,
}

impl Default for DekaLock {
    fn default() -> Self {
        Self {
            lockfile_version: 1,
            node: EcosystemSection::default(),
            php: EcosystemSection::default(),
        }
    }
}

pub type LockEntry = (String, String, Value, String);

fn lock_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    Some(cwd.join(LOCKFILE_NAME))
}

pub fn read_lockfile() -> DekaLock {
    match lock_path() {
        Some(path) => read_lockfile_at(&path),
        None => DekaLock::default(),
    }
}

pub fn read_lockfile_at(path: &Path) -> DekaLock {
    if path.exists() {
        if let Ok(mut file) = File::open(path) {
            let mut buf = String::new();
            if file.read_to_string(&mut buf).is_ok() {
                if let Ok(parsed) = serde_json::from_str::<DekaLock>(&buf) {
                    return parsed;
                }
            }
        }
    }
    DekaLock::default()
}

pub fn write_lockfile(lock: &DekaLock) -> Result<()> {
    let path = lock_path().ok_or_else(|| anyhow!("lock path not available"))?;
    write_lockfile_at(&path, lock)
}

pub fn write_lockfile_at(path: &Path, lock: &DekaLock) -> Result<()> {
    let file = File::create(&path)?;
    serde_json::to_writer_pretty(file, lock)?;
    Ok(())
}

pub fn update_lock_entry(
    ecosystem: &str,
    name: &str,
    descriptor: String,
    resolved: String,
    metadata: Value,
    integrity: String,
) -> Result<()> {
    let path = lock_path().ok_or_else(|| anyhow!("lock path not available"))?;
    update_lock_entry_at(
        &path, ecosystem, name, descriptor, resolved, metadata, integrity,
    )
}

pub fn update_lock_entry_at(
    path: &Path,
    ecosystem: &str,
    name: &str,
    descriptor: String,
    resolved: String,
    metadata: Value,
    integrity: String,
) -> Result<()> {
    let mut lock = read_lockfile_at(path);
    let entry = (descriptor, resolved, metadata, integrity);
    match ecosystem {
        "node" => {
            lock.node.packages.insert(name.to_string(), entry);
        }
        "php" => {
            lock.php.packages.insert(name.to_string(), entry);
        }
        other => {
            return Err(anyhow!("unknown ecosystem {}", other));
        }
    }
    write_lockfile_at(path, &lock)?;
    Ok(())
}
