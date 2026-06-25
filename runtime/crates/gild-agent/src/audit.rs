use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::group_gid;

pub(crate) fn unique_tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(format!(
        ".tmp.{}.{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    PathBuf::from(tmp)
}

pub(crate) fn open_audit_log(path: &Path) -> Result<fs::File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o640)
        .open(path)
        .with_context(|| format!("open audit log {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o640))
        .with_context(|| format!("chmod 0640 {}", path.display()))?;
    chown_path(path, audit_log_owner())?;
    Ok(file)
}

pub(crate) fn chown_path(path: &Path, owner: Option<(u32, u32)>) -> Result<()> {
    let Some((uid, gid)) = owner else {
        return Ok(());
    };
    let c_path =
        std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).context("path contains NUL")?;
    let rc = unsafe { libc::chown(c_path.as_ptr(), uid, gid) };
    if rc != 0 {
        return Err(io::Error::last_os_error())
            .with_context(|| format!("chown {uid}:{gid} {}", path.display()));
    }
    Ok(())
}

fn audit_log_owner() -> Option<(u32, u32)> {
    if unsafe { libc::geteuid() } != 0 {
        return None;
    }
    let gid = group_gid("adm").ok().flatten().unwrap_or(0);
    Some((0, gid))
}

pub(crate) fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    hex_encode(&digest)
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(crate) fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
