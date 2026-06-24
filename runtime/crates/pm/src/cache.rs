use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use dirs::home_dir;
use sha2::{Digest, Sha512};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::Builder;

pub struct CachePaths {
    pub global: PathBuf,
    pub node: PathBuf,
    pub archive: PathBuf,
    pub meta: PathBuf,
    pub tmp: PathBuf,
    pub node_modules: PathBuf,
}

impl CachePaths {
    pub fn new() -> Result<Self> {
        let home = home_dir().ok_or_else(|| anyhow!("home directory not found"))?;
        let global = home.join(".config").join("deka").join("pm").join("cache");
        let node = global.join("node");
        let archive = global.join("archives");
        let meta = global.join("meta");
        let tmp = global.join("tmp");
        let node_modules = env::current_dir()?.join("node_modules");
        Ok(Self {
            global,
            node,
            archive,
            meta,
            tmp,
            node_modules,
        })
    }

    pub fn ensure(&self) -> Result<()> {
        for dir in [
            &self.global,
            &self.node,
            &self.archive,
            &self.meta,
            &self.tmp,
        ] {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create cache directory {}", dir.display()))?;
        }
        Ok(())
    }

    pub fn cache_dir(&self, key: &str) -> PathBuf {
        self.node.join(key)
    }

    pub fn archive_path(&self, key: &str) -> PathBuf {
        self.archive.join(format!("{key}.tgz"))
    }

    pub fn metadata_path(&self, key: &str) -> PathBuf {
        self.meta.join(format!("{key}.json"))
    }

    pub fn project_path_for(&self, name: &str) -> PathBuf {
        let segments: Vec<&str> = name.split('/').collect();
        segments
            .iter()
            .fold(self.node_modules.clone(), |acc, segment| acc.join(segment))
    }

    pub fn project_path_for_lock_key(&self, name: &str, lock_key: Option<&str>) -> PathBuf {
        if let Some(key) = lock_key {
            let segments: Vec<&str> = key.split('/').collect();
            if segments.len() > 1 {
                let mut path = self.node_modules.clone();
                for segment in &segments[..segments.len() - 1] {
                    path = path.join(segment);
                }
                path = path.join("node_modules");
                let name_segments: Vec<&str> = name.split('/').collect();
                for segment in name_segments {
                    path = path.join(segment);
                }
                return path;
            }
        }
        self.project_path_for(name)
    }
}

pub fn sanitize_name(name: &str) -> String {
    name.replace('/', "+")
}

pub fn cache_key(name: &str, version: &str) -> String {
    format!("{}@{}", sanitize_name(name), version)
}

pub async fn download_tarball(url: &str) -> Result<Vec<u8>> {
    let response = reqwest::get(url)
        .await
        .with_context(|| format!("failed to download tarball {}", url))?;
    if !response.status().is_success() {
        anyhow::bail!("tarball request failed ({})", response.status())
    }
    Ok(response.bytes().await?.to_vec())
}

pub fn compute_sha512(data: &[u8]) -> String {
    let mut hasher = Sha512::new();
    hasher.update(data);
    format!("sha512-{}", STANDARD.encode(hasher.finalize()))
}

pub fn extract_tarball(archive_path: &Path, destination: &Path, tmp_root: &Path) -> Result<()> {
    let tmp = Builder::new()
        .prefix("deka-")
        .tempdir_in(tmp_root)
        .context("failed to create temp directory")?;
    let tmp_path = tmp.path();

    let status = Command::new("tar")
        .args(["-xzf", archive_path.to_string_lossy().as_ref()])
        .arg("-C")
        .arg(tmp_path)
        .status()
        .context("failed to spawn tar")?;

    if !status.success() {
        anyhow::bail!("tar command failed with {}", status);
    }

    if destination.exists() {
        fs::remove_dir_all(destination).ok();
    }

    let entries = fs::read_dir(tmp_path)?.collect::<io::Result<Vec<_>>>()?;
    if entries.len() == 1 {
        let entry = &entries[0];
        let name = entry.file_name();
        if name == "package" {
            fs::rename(entry.path(), destination)?;
            return Ok(());
        }
    }

    fs::create_dir_all(destination)?;
    for entry in entries {
        let src = entry.path();
        let dst = destination.join(entry.file_name());
        fs::rename(src, dst)?;
    }
    Ok(())
}

pub fn copy_package(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    // Use hardlinks for instant "copying" - like Bun does
    hardlink_dir(source, destination).context("failed to link package")
}

fn hardlink_dir(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = destination.join(entry.file_name());

        if src_path.is_dir() {
            hardlink_dir(&src_path, &dst_path)?;
        } else {
            // Try clonefile (macOS APFS), then hardlink, then copy
            if !try_clonefile(&src_path, &dst_path) {
                if fs::hard_link(&src_path, &dst_path).is_err() {
                    fs::copy(&src_path, &dst_path)?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn try_clonefile(source: &Path, destination: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let src = match CString::new(source.as_os_str().as_bytes()) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let dst = match CString::new(destination.as_os_str().as_bytes()) {
        Ok(d) => d,
        Err(_) => return false,
    };

    // clonefile(2) - creates a copy-on-write clone on APFS
    unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), 0) == 0 }
}

#[cfg(not(target_os = "macos"))]
fn try_clonefile(_source: &Path, _destination: &Path) -> bool {
    false
}

pub fn write_metadata(path: &Path, metadata: &serde_json::Value) -> Result<()> {
    let file = File::create(path)?;
    serde_json::to_writer_pretty(file, metadata)?;
    Ok(())
}

pub fn read_metadata(path: &Path) -> Option<serde_json::Value> {
    let mut file = File::open(path).ok()?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}
