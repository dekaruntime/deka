use age::secrecy::ExposeSecret;
use clap::{Args, Subcommand};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::Result;

const DEFAULT_MASTER_KEY_PATH: &str = "/etc/gild/vault-master.key";
const DEFAULT_PLAINTEXT_STATE_PATH: &str = "/run/gild-vault/keys.json";
const DEFAULT_ENCRYPTED_STATE_PATH: &str = "/var/lib/gild-vault/keys.age";

#[derive(Debug, Args)]
pub struct VaultArgs {
    #[command(subcommand)]
    command: VaultCommand,
}

#[derive(Debug, Subcommand)]
enum VaultCommand {
    /// Get a secret value.
    Get { key: String },
    /// Generate the file-backed vault master key.
    Init {
        /// Overwrite an existing master key.
        #[arg(long)]
        force: bool,
        #[arg(long, hide = true)]
        master_key_path: Option<PathBuf>,
    },
    /// Encrypt the legacy plaintext tmpfs vault state into persistent storage.
    MigrateFromPlaintext {
        #[arg(long, hide = true)]
        master_key_path: Option<PathBuf>,
        #[arg(long, hide = true)]
        plaintext_path: Option<PathBuf>,
        #[arg(long, hide = true)]
        state_path: Option<PathBuf>,
    },
}

pub async fn run(args: VaultArgs) -> Result<()> {
    match args.command {
        VaultCommand::Get { .. } => {
            println!("not yet implemented in this skeleton PR");
            Ok(())
        }
        VaultCommand::Init {
            force,
            master_key_path,
        } => {
            let path = master_key_path.unwrap_or_else(|| PathBuf::from(DEFAULT_MASTER_KEY_PATH));
            let public_key = init_master_key(&path, force)?;
            println!("vault master key initialized at {}", path.display());
            println!("public key fingerprint: {public_key}");
            Ok(())
        }
        VaultCommand::MigrateFromPlaintext {
            master_key_path,
            plaintext_path,
            state_path,
        } => {
            let master_key_path =
                master_key_path.unwrap_or_else(|| PathBuf::from(DEFAULT_MASTER_KEY_PATH));
            let plaintext_path =
                plaintext_path.unwrap_or_else(|| PathBuf::from(DEFAULT_PLAINTEXT_STATE_PATH));
            let state_path =
                state_path.unwrap_or_else(|| PathBuf::from(DEFAULT_ENCRYPTED_STATE_PATH));
            let count = migrate_from_plaintext(&master_key_path, &plaintext_path, &state_path)?;
            println!(
                "migrated {count} keys from {} to {}",
                plaintext_path.display(),
                state_path.display()
            );
            println!(
                "now restart gg.tana.gild-vault.service. Verify keys present. Then delete /run/gild-vault/keys.json manually."
            );
            Ok(())
        }
    }
}

fn init_master_key(path: &Path, force: bool) -> Result<String> {
    if path.exists() && !force {
        return Err(error(format!(
            "{} already exists; pass --force to overwrite",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o750))?;
        try_chown(parent, Some("root"), Some("gild"));
    }

    let identity = age::x25519::Identity::generate();
    let public_key = identity.to_public().to_string();
    let mut options = OpenOptions::new();
    options.write(true).mode(0o400);
    if force {
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }

    let mut file = options.open(path)?;
    try_chown(path, Some("gild-vault"), None);
    writeln!(file, "{}", identity.to_string().expose_secret())?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    try_chown(path, Some("gild-vault"), None);
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(public_key)
}

fn migrate_from_plaintext(
    master_key_path: &Path,
    plaintext_path: &Path,
    state_path: &Path,
) -> Result<usize> {
    let identity = load_master_key(master_key_path)?;
    let plaintext = match fs::read(plaintext_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "{} not present; nothing to migrate",
                plaintext_path.display()
            );
            return Ok(0);
        }
        Err(err) => return Err(Box::new(err)),
    };
    let keys: HashMap<String, String> = serde_json::from_slice(&plaintext)?;
    persist_encrypted_state(state_path, &identity.to_public(), &keys)?;
    let loaded = load_encrypted_state(state_path, &identity)?;
    if loaded != keys {
        return Err(error("migration round-trip verification failed"));
    }
    Ok(keys.len())
}

fn load_master_key(path: &Path) -> Result<age::x25519::Identity> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(error(format!(
                "missing gild-vault master key at {}; run `gild vault init` first",
                path.display()
            )));
        }
        Err(err) => return Err(Box::new(err)),
    };
    let identity = contents
        .lines()
        .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .ok_or_else(|| error(format!("{} does not contain a master key", path.display())))?
        .trim()
        .parse::<age::x25519::Identity>()?;
    Ok(identity)
}

fn load_encrypted_state(
    path: &Path,
    identity: &age::x25519::Identity,
) -> Result<HashMap<String, String>> {
    let ciphertext = fs::read(path)?;
    let plaintext = age::decrypt(identity, &ciphertext)?;
    Ok(serde_json::from_slice(&plaintext)?)
}

fn persist_encrypted_state(
    path: &Path,
    recipient: &age::x25519::Recipient,
    keys: &HashMap<String, String>,
) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| error("state path must have a parent"))?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o750))?;
    try_chown(parent, Some("gild-vault"), Some("gild"));

    let plaintext = serde_json::to_vec(keys)?;
    let ciphertext = age::encrypt(recipient, &plaintext)?;
    let tmp_path = path.with_extension(format!("age.tmp.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp_path)?;
    try_chown(&tmp_path, Some("gild-vault"), None);
    file.write_all(&ciphertext)?;
    file.sync_all()?;
    fs::rename(&tmp_path, path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    try_chown(path, Some("gild-vault"), None);
    sync_dir(parent)?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

fn try_chown(path: &Path, user: Option<&str>, group: Option<&str>) {
    let uid = user.and_then(uid_for_user).unwrap_or(u32::MAX);
    let gid = group.and_then(gid_for_group).unwrap_or(u32::MAX);
    if uid == u32::MAX && gid == u32::MAX {
        return;
    }
    let bytes = path.as_os_str().as_bytes();
    let mut c_path = Vec::with_capacity(bytes.len() + 1);
    c_path.extend_from_slice(bytes);
    c_path.push(0);
    unsafe {
        libc::chown(c_path.as_ptr().cast(), uid, gid);
    }
}

fn uid_for_user(user: &str) -> Option<u32> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        let raw_uid = fields.next()?;
        if name == user {
            raw_uid.parse::<u32>().ok()
        } else {
            None
        }
    })
}

fn gid_for_group(group: &str) -> Option<u32> {
    let groups = fs::read_to_string("/etc/group").ok()?;
    groups.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        let raw_gid = fields.next()?;
        if name == group {
            raw_gid.parse::<u32>().ok()
        } else {
            None
        }
    })
}

fn error(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_refuses_to_overwrite_without_force() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault-master.key");

        let first = init_master_key(&path, false).unwrap();
        let err = init_master_key(&path, false).unwrap_err();

        assert!(err.to_string().contains("--force"));
        assert_eq!(
            load_master_key(&path).unwrap().to_public().to_string(),
            first
        );
    }

    #[test]
    fn init_force_overwrites_master_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault-master.key");

        let first = init_master_key(&path, false).unwrap();
        let second = init_master_key(&path, true).unwrap();

        assert_ne!(first, second);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o400
        );
    }

    #[test]
    fn migration_encrypts_plaintext_and_verifies_round_trip() {
        let dir = tempdir().unwrap();
        let master_path = dir.path().join("vault-master.key");
        let plaintext_path = dir.path().join("keys.json");
        let state_path = dir.path().join("state").join("keys.age");
        init_master_key(&master_path, false).unwrap();
        let keys = HashMap::from([
            ("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string()),
            ("AGENT_KHALID_TOKEN".to_string(), "agent-token".to_string()),
        ]);
        fs::write(&plaintext_path, serde_json::to_vec(&keys).unwrap()).unwrap();

        let count = migrate_from_plaintext(&master_path, &plaintext_path, &state_path).unwrap();
        let identity = load_master_key(&master_path).unwrap();
        let loaded = load_encrypted_state(&state_path, &identity).unwrap();

        assert_eq!(count, 2);
        assert_eq!(loaded, keys);
        assert!(plaintext_path.exists());
        assert_ne!(
            fs::read(&state_path).unwrap(),
            fs::read(&plaintext_path).unwrap()
        );
    }
}
