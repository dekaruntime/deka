use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const LOCKFILE_NAME: &str = "deka.lock";

#[derive(Debug, Serialize, Deserialize)]
pub struct DekaLock {
    #[serde(rename = "lockfileVersion")]
    pub lockfile_version: u32,
    pub packages: BTreeMap<String, LockEntry>,
}

impl Default for DekaLock {
    fn default() -> Self {
        Self {
            lockfile_version: 1,
            packages: BTreeMap::new(),
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
                return parse_lockfile(&buf);
            }
        }
    }
    DekaLock::default()
}

fn parse_lockfile(contents: &str) -> DekaLock {
    if let Ok(parsed) = serde_json::from_str::<DekaLock>(contents) {
        return parsed;
    }

    let Ok(value) = serde_json::from_str::<Value>(contents) else {
        return DekaLock::default();
    };
    let lockfile_version = value
        .get("lockfileVersion")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .unwrap_or(1);
    let packages = value
        .get("php")
        .and_then(|section| section.get("packages"))
        .and_then(|packages| {
            serde_json::from_value::<BTreeMap<String, LockEntry>>(packages.clone()).ok()
        })
        .unwrap_or_default();

    DekaLock {
        lockfile_version,
        packages,
    }
}

pub fn write_lockfile(lock: &DekaLock) -> Result<()> {
    let path = lock_path().ok_or_else(|| anyhow!("lock path not available"))?;
    write_lockfile_at(&path, lock)
}

pub fn write_lockfile_at(path: &Path, lock: &DekaLock) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("lock path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.tmp-{}-{}",
        LOCKFILE_NAME,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    let mut file = File::create(&temp)?;
    serde_json::to_writer_pretty(&mut file, lock)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    Ok(())
}

pub fn update_lock_entry(
    name: &str,
    descriptor: String,
    resolved: String,
    metadata: Value,
    integrity: String,
) -> Result<()> {
    let path = lock_path().ok_or_else(|| anyhow!("lock path not available"))?;
    update_lock_entry_at(&path, name, descriptor, resolved, metadata, integrity)
}

pub fn update_lock_entry_at(
    path: &Path,
    name: &str,
    descriptor: String,
    resolved: String,
    metadata: Value,
    integrity: String,
) -> Result<()> {
    let mut lock = read_lockfile_at(path);
    let entry = (descriptor, resolved, metadata, integrity);
    lock.packages.insert(name.to_string(), entry);
    write_lockfile_at(path, &lock)?;
    Ok(())
}
