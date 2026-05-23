use crate::{PeerCredentials, RotateHmacRequest, ServiceReply, timestamp_millis, validate_slug};
use anyhow::{Context, Result, bail};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const HMAC_KEY_FILE: &str = "hmac.key";
const ROTATED_AT_FILE: &str = ".rotated-at";

pub(crate) fn handle_hmac_rotate_request(
    request: &RotateHmacRequest,
    peer: PeerCredentials,
    agents_root: &Path,
    audit_path: &Path,
    owner: Option<(u32, u32)>,
) -> ServiceReply {
    match rotate_hmac(request, peer, agents_root, audit_path, owner) {
        Ok(fingerprint) => ServiceReply::json_value(
            200,
            serde_json::json!({
                "ok": true,
                "new_key_fingerprint": fingerprint
            }),
        ),
        Err(err) => ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() })),
    }
}

fn rotate_hmac(
    request: &RotateHmacRequest,
    peer: PeerCredentials,
    agents_root: &Path,
    audit_path: &Path,
    owner: Option<(u32, u32)>,
) -> Result<String> {
    if request.op != "hmac_rotate" {
        bail!("op must be hmac_rotate");
    }
    validate_slug(&request.slug)?;

    let key = generate_hmac_key();
    let fingerprint = sha256_fingerprint(key.as_bytes());
    let agent_dir = agents_root.join(&request.slug);
    let key_path = agent_dir.join(HMAC_KEY_FILE);

    ensure_private_dir(&agent_dir, owner)?;
    atomic_write_private(&key_path, key.as_bytes(), owner)?;
    touch_rotated_at(&agent_dir.join(ROTATED_AT_FILE), owner)?;
    write_audit(audit_path, &request.slug, &fingerprint, peer)?;

    Ok(fingerprint)
}

fn generate_hmac_key() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex_encode(&bytes)
}

fn sha256_fingerprint(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    format!("sha256:{}", hex_encode(&digest))
}

fn ensure_private_dir(path: &Path, owner: Option<(u32, u32)>) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("chmod 0700 {}", path.display()))?;
    chown_path(path, owner)
}

fn atomic_write_private(path: &Path, contents: &[u8], owner: Option<(u32, u32)>) -> Result<()> {
    let tmp_path = tmp_path(path);
    {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp_path)
            .with_context(|| format!("open {}", tmp_path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", tmp_path.display()))?;
    }
    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 0600 {}", tmp_path.display()))?;
    chown_path(&tmp_path, owner)?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("rename {} to {}", tmp_path.display(), path.display()))?;
    Ok(())
}

fn touch_rotated_at(path: &Path, owner: Option<(u32, u32)>) -> Result<()> {
    let timestamp = timestamp_millis().to_string();
    atomic_write_private(path, timestamp.as_bytes(), owner)
}

fn write_audit(path: &Path, slug: &str, fingerprint: &str, peer: PeerCredentials) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open audit log {}", path.display()))?;
    let line = serde_json::json!({
        "ts": timestamp_millis().to_string(),
        "op": "hmac_rotate",
        "slug": slug,
        "new_fingerprint": fingerprint,
        "by_uid": peer.uid,
        "by_pid": peer.pid
    });
    writeln!(file, "{line}").context("write audit log")
}

fn chown_path(path: &Path, owner: Option<(u32, u32)>) -> Result<()> {
    let Some((uid, gid)) = owner else {
        return Ok(());
    };
    let c_path =
        std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).context("path contains NUL")?;
    let rc = unsafe { libc::chown(c_path.as_ptr(), uid, gid) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("chown {uid}:{gid} {}", path.display()));
    }
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn temp_root(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("gild-agent-hmac-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn peer() -> PeerCredentials {
        PeerCredentials {
            pid: 123,
            uid: 456,
            gid: 789,
        }
    }

    fn request(slug: &str) -> RotateHmacRequest {
        RotateHmacRequest {
            op: "hmac_rotate".to_string(),
            slug: slug.to_string(),
        }
    }

    #[test]
    fn slug_regex_accepts_only_agent_slugs() {
        assert!(validate_slug("agent-ab").is_ok());
        assert!(validate_slug("agent-a1-b2").is_ok());
        assert!(validate_slug("agent-a234567890123456789012345678901").is_ok());

        assert!(validate_slug("agent-a").is_err());
        assert!(validate_slug("Agent-ab").is_err());
        assert!(validate_slug("agent-1a").is_err());
        assert!(validate_slug("agent-a_b").is_err());
        assert!(validate_slug("other-agent-ab").is_err());
        assert!(validate_slug("agent-a2345678901234567890123456789012").is_err());
    }

    #[test]
    fn rotate_twice_changes_key_and_keeps_private_mode() {
        let root = temp_root("twice");
        let audit = root.join("audit.log");
        let owner = Some((unsafe { libc::geteuid() }, unsafe { libc::getegid() }));

        let first = rotate_hmac(&request("agent-foo"), peer(), &root, &audit, owner).unwrap();
        let key_path = root.join("agent-foo").join(HMAC_KEY_FILE);
        let first_key = fs::read_to_string(&key_path).unwrap();
        let first_meta = fs::metadata(&key_path).unwrap();

        let second = rotate_hmac(&request("agent-foo"), peer(), &root, &audit, owner).unwrap();
        let second_key = fs::read_to_string(&key_path).unwrap();
        let second_meta = fs::metadata(&key_path).unwrap();

        assert_ne!(first_key, second_key);
        assert_ne!(first, second);
        assert_eq!(first_key.len(), 64);
        assert_eq!(second_key.len(), 64);
        assert_eq!(first_meta.mode() & 0o777, 0o600);
        assert_eq!(second_meta.mode() & 0o777, 0o600);
        assert_eq!(second_meta.uid(), unsafe { libc::geteuid() });
        assert!(root.join("agent-foo").join(ROTATED_AT_FILE).exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bad_slug_returns_error_without_writing() {
        let root = temp_root("bad-slug");
        let audit = root.join("audit.log");

        let reply =
            handle_hmac_rotate_request(&request("../agent-foo"), peer(), &root, &audit, None);

        assert_eq!(reply.status, 400);
        assert!(!root.exists());
    }

    #[test]
    fn fingerprint_is_sha256_hex_of_key_contents() {
        let root = temp_root("fingerprint");
        let audit = root.join("audit.log");

        let fingerprint = rotate_hmac(&request("agent-bar"), peer(), &root, &audit, None).unwrap();
        let key = fs::read(root.join("agent-bar").join(HMAC_KEY_FILE)).unwrap();

        assert_eq!(fingerprint, sha256_fingerprint(&key));
        assert!(fingerprint.starts_with("sha256:"));
        assert_eq!(fingerprint.len(), "sha256:".len() + 64);

        let _ = fs::remove_dir_all(root);
    }
}
