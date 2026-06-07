use age::secrecy::ExposeSecret;
use clap::{Args, Subcommand};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use time::{Duration, OffsetDateTime};

use crate::Result;

const DEFAULT_MASTER_KEY_PATH: &str = "/etc/gild/vault-master.key";
const DEFAULT_REPLICATION_TOKEN_PATH: &str = "/etc/gild/vault-replication-token";
const DEFAULT_PLAINTEXT_STATE_PATH: &str = "/run/gild-vault/keys.json";
const DEFAULT_ENCRYPTED_STATE_PATH: &str = "/var/lib/gild-vault/keys.age";
const DEFAULT_VAULT_PROXY_CA_CERT_PATH: &str = "/etc/gild/vault-proxy-ca.pem";
const DEFAULT_VAULT_PROXY_CA_KEY_PATH: &str = "/etc/gild/vault-proxy-ca.key";
const PROMOTION_EPOCH_BUMP: u64 = 1000;
const VAULT_PROXY_CERT_LIFETIME_DAYS: i64 = 365;

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
    /// Generate the shared vault replication bearer token.
    InitReplicationToken {
        /// Overwrite an existing replication token.
        #[arg(long)]
        force: bool,
        /// Owner for the token file.
        #[arg(long, default_value = "gild-vault")]
        owner: String,
        #[arg(long, hide = true)]
        path: Option<PathBuf>,
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
    /// Mark this node as the operator-confirmed canonical authoritative.
    ForceAuthoritative {
        #[arg(long, hide = true)]
        state_path: Option<PathBuf>,
    },
    /// Manage the local gild-vault-proxy mTLS certificate authority.
    Ca {
        #[command(subcommand)]
        command: VaultCaCommand,
    },
}

#[derive(Debug, Subcommand)]
enum VaultCaCommand {
    /// Generate the self-signed local vault proxy CA, valid for 365 days.
    ///
    /// Certs expire after 365 days. Plan to re-issue (or automate rotation via deka#65) before expiry.
    Init {
        /// Overwrite an existing CA certificate or key.
        #[arg(long)]
        force: bool,
        #[arg(long, hide = true)]
        ca_cert_path: Option<PathBuf>,
        #[arg(long, hide = true)]
        ca_key_path: Option<PathBuf>,
    },
    /// Issue a 365-day client certificate signed by the local CA and print it to stdout.
    ///
    /// Certs expire after 365 days. Plan to re-issue (or automate rotation via deka#65) before expiry.
    Issue {
        cn: String,
        #[arg(long, hide = true)]
        server: bool,
        #[arg(long, hide = true)]
        ca_cert_path: Option<PathBuf>,
        #[arg(long, hide = true)]
        ca_key_path: Option<PathBuf>,
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
        VaultCommand::InitReplicationToken { force, owner, path } => {
            let path = path.unwrap_or_else(|| PathBuf::from(DEFAULT_REPLICATION_TOKEN_PATH));
            init_replication_token(&path, force, &owner)?;
            println!("vault replication token initialized at {}", path.display());
            println!("copy this exact file to each replica during provisioning");
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
        VaultCommand::ForceAuthoritative { state_path } => {
            let state_path =
                state_path.unwrap_or_else(|| PathBuf::from(DEFAULT_ENCRYPTED_STATE_PATH));
            let meta = force_authoritative(&state_path)?;
            println!(
                "vault authoritative fence reset at {}: epoch={}, version={}",
                replication_meta_path(&state_path).display(),
                meta.epoch,
                meta.version
            );
            println!("restart gg.tana.gild-vault.service on this node; demote all other nodes.");
            Ok(())
        }
        VaultCommand::Ca { command } => match command {
            VaultCaCommand::Init {
                force,
                ca_cert_path,
                ca_key_path,
            } => {
                let ca_cert_path =
                    ca_cert_path.unwrap_or_else(|| PathBuf::from(DEFAULT_VAULT_PROXY_CA_CERT_PATH));
                let ca_key_path =
                    ca_key_path.unwrap_or_else(|| PathBuf::from(DEFAULT_VAULT_PROXY_CA_KEY_PATH));
                init_vault_proxy_ca(&ca_cert_path, &ca_key_path, force)?;
                println!("vault proxy CA initialized at {}", ca_cert_path.display());
                println!("vault proxy CA key written to {}", ca_key_path.display());
                Ok(())
            }
            VaultCaCommand::Issue {
                cn,
                server,
                ca_cert_path,
                ca_key_path,
            } => {
                let ca_cert_path =
                    ca_cert_path.unwrap_or_else(|| PathBuf::from(DEFAULT_VAULT_PROXY_CA_CERT_PATH));
                let ca_key_path =
                    ca_key_path.unwrap_or_else(|| PathBuf::from(DEFAULT_VAULT_PROXY_CA_KEY_PATH));
                let usage = if server {
                    VaultProxyCertUsage::Server
                } else {
                    VaultProxyCertUsage::Client
                };
                let issued = issue_vault_proxy_cert(&ca_cert_path, &ca_key_path, &cn, usage)?;
                print!("{issued}");
                Ok(())
            }
        },
    }
}

fn init_replication_token(path: &Path, force: bool, owner: &str) -> Result<()> {
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

    let token = random_hex_token()?;
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
    try_chown(path, Some(owner), None);
    writeln!(file, "{token}")?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    try_chown(path, Some(owner), None);
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

fn random_hex_token() -> Result<String> {
    let mut bytes = [0_u8; 16];
    let mut random = fs::File::open("/dev/urandom")?;
    std::io::Read::read_exact(&mut random, &mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn init_vault_proxy_ca(ca_cert_path: &Path, ca_key_path: &Path, force: bool) -> Result<()> {
    if !force {
        for path in [ca_cert_path, ca_key_path] {
            if path.exists() {
                return Err(error(format!(
                    "{} already exists; pass --force to overwrite",
                    path.display()
                )));
            }
        }
    }

    ensure_gild_config_dir(ca_cert_path)?;
    ensure_gild_config_dir(ca_key_path)?;

    let mut params = vault_proxy_cert_params(vec!["tana-gild-vault-proxy-ca".to_string()]);
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "tana-gild-vault-proxy-ca");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let cert = Certificate::from_params(params)?;
    write_private_file(ca_key_path, &cert.serialize_private_key_pem(), force)?;
    write_private_file(ca_cert_path, &cert.serialize_pem()?, force)?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum VaultProxyCertUsage {
    Client,
    Server,
}

fn issue_vault_proxy_cert(
    ca_cert_path: &Path,
    ca_key_path: &Path,
    cn: &str,
    usage: VaultProxyCertUsage,
) -> Result<String> {
    validate_cn(cn)?;
    let ca_cert_pem = fs::read_to_string(ca_cert_path)?;
    let ca_key_pem = fs::read_to_string(ca_key_path)?;
    let ca_key = KeyPair::from_pem(&ca_key_pem)?;
    let ca_params = CertificateParams::from_ca_cert_pem(&ca_cert_pem, ca_key)?;
    let ca_cert = Certificate::from_params(ca_params)?;

    let mut params = vault_proxy_cert_params(vec![cn.to_string()]);
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, cn);
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![match usage {
        VaultProxyCertUsage::Client => ExtendedKeyUsagePurpose::ClientAuth,
        VaultProxyCertUsage::Server => ExtendedKeyUsagePurpose::ServerAuth,
    }];
    let cert = Certificate::from_params(params)?;
    Ok(format!(
        "{}{}",
        cert.serialize_pem_with_signer(&ca_cert)?,
        cert.serialize_private_key_pem()
    ))
}

fn vault_proxy_cert_params(subject_alt_names: Vec<String>) -> CertificateParams {
    let mut params = CertificateParams::default();
    params.subject_alt_names = subject_alt_names
        .into_iter()
        .map(|name| match name.parse() {
            Ok(ip) => SanType::IpAddress(ip),
            Err(_) => SanType::DnsName(name),
        })
        .collect();
    apply_vault_proxy_cert_lifetime(&mut params);
    params
}

fn apply_vault_proxy_cert_lifetime(params: &mut CertificateParams) {
    let not_before = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("zero nanosecond timestamp is valid");
    params.not_before = not_before;
    params.not_after = not_before + Duration::days(VAULT_PROXY_CERT_LIFETIME_DAYS);
}

fn validate_cn(cn: &str) -> Result<()> {
    if cn.is_empty()
        || cn.len() > 128
        || !cn
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        Err(error(
            "client certificate CN must be 1-128 chars of [A-Za-z0-9._-]",
        ))
    } else {
        Ok(())
    }
}

fn ensure_gild_config_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o750))?;
        try_chown(parent, Some("root"), Some("gild"));
        sync_dir(parent)?;
    }
    Ok(())
}

fn write_private_file(path: &Path, contents: &str, force: bool) -> Result<()> {
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
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    try_chown(path, Some("root"), None);
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
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

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct ReplicationMeta {
    epoch: u64,
    version: u64,
    fenced: bool,
}

fn force_authoritative(state_path: &Path) -> Result<ReplicationMeta> {
    let meta_path = replication_meta_path(state_path);
    let mut meta = load_replication_meta(&meta_path)?;
    meta.epoch = meta.epoch.saturating_add(PROMOTION_EPOCH_BUMP);
    meta.fenced = false;
    persist_replication_meta(&meta_path, &meta)?;
    Ok(meta)
}

fn replication_meta_path(state_path: &Path) -> PathBuf {
    state_path.with_extension("replication.json")
}

fn load_replication_meta(path: &Path) -> Result<ReplicationMeta> {
    match fs::read(path) {
        Ok(bytes) if bytes.is_empty() => Ok(ReplicationMeta {
            epoch: 0,
            version: 0,
            fenced: false,
        }),
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(ReplicationMeta {
            epoch: 0,
            version: 0,
            fenced: false,
        }),
        Err(err) => Err(Box::new(err)),
    }
}

fn persist_replication_meta(path: &Path, meta: &ReplicationMeta) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| error("replication metadata path must have a parent"))?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o750))?;
    try_chown(parent, Some("gild-vault"), Some("gild"));

    let tmp_path = path.with_extension(format!("replication.json.tmp.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp_path)?;
    try_chown(&tmp_path, Some("gild-vault"), None);
    serde_json::to_writer(&mut file, meta)?;
    file.write_all(b"\n")?;
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
    use x509_parser::prelude::{FromDer, X509Certificate};

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

    #[test]
    fn force_authoritative_bumps_epoch_and_clears_fence() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("keys.age");
        let meta_path = replication_meta_path(&state_path);
        persist_replication_meta(
            &meta_path,
            &ReplicationMeta {
                epoch: 42,
                version: 9,
                fenced: true,
            },
        )
        .unwrap();

        let meta = force_authoritative(&state_path).unwrap();
        let persisted = load_replication_meta(&meta_path).unwrap();

        assert_eq!(meta.epoch, 42 + PROMOTION_EPOCH_BUMP);
        assert_eq!(meta.version, 9);
        assert!(!meta.fenced);
        assert_eq!(persisted.epoch, meta.epoch);
        assert!(!persisted.fenced);
    }

    #[test]
    fn vault_proxy_ca_init_and_issue_round_trip() {
        let dir = tempdir().unwrap();
        let ca_cert_path = dir.path().join("vault-proxy-ca.pem");
        let ca_key_path = dir.path().join("vault-proxy-ca.key");

        init_vault_proxy_ca(&ca_cert_path, &ca_key_path, false).unwrap();
        let issued = issue_vault_proxy_cert(
            &ca_cert_path,
            &ca_key_path,
            "storefront-do-01",
            VaultProxyCertUsage::Client,
        )
        .unwrap();

        assert!(issued.contains("BEGIN CERTIFICATE"));
        assert!(issued.contains("BEGIN PRIVATE KEY"));
        assert_eq!(
            fs::metadata(&ca_cert_path).unwrap().permissions().mode() & 0o777,
            0o400
        );
        assert_eq!(
            fs::metadata(&ca_key_path).unwrap().permissions().mode() & 0o777,
            0o400
        );
        assert!(init_vault_proxy_ca(&ca_cert_path, &ca_key_path, false).is_err());

        let cert_pem = issued
            .split("-----END CERTIFICATE-----")
            .next()
            .unwrap()
            .to_string()
            + "-----END CERTIFICATE-----\n";
        let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).unwrap();
        let (_, cert) = X509Certificate::from_der(&pem.contents).unwrap();
        let cn = cert
            .subject()
            .iter_common_name()
            .next()
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(cn, "storefront-do-01");

        let now = OffsetDateTime::now_utc();
        let not_after = cert.validity().not_after.to_datetime();
        assert!(
            not_after
                <= now + Duration::days(VAULT_PROXY_CERT_LIFETIME_DAYS) + Duration::minutes(5),
            "issued cert expires too far in the future: {not_after}"
        );
        assert!(
            not_after > now + Duration::days(VAULT_PROXY_CERT_LIFETIME_DAYS - 1),
            "issued cert lifetime is unexpectedly short: {not_after}"
        );
    }
}
