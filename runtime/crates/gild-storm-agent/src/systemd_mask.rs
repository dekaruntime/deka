use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::supervisor::audit;

const DROPIN_NAME: &str = "storm-mask.conf";
const DROPIN_BODY: &[u8] = b"[Service]\nRestart=no\n";
const SYSTEM_ROOT: &str = "/etc/systemd/system";
const RUN_ROOT: &str = "/run/gild-storm-agent";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaskOwner {
    pub service: String,
    pub run_id: String,
}

#[derive(Debug, Clone)]
pub struct MaskGuard {
    pub service: String,
    pub run_id: String,
}

#[derive(Debug, Clone)]
pub struct SystemdMaskConfig {
    system_root: PathBuf,
    run_root: PathBuf,
    systemctl: PathBuf,
}

impl SystemdMaskConfig {
    fn production() -> Self {
        Self {
            system_root: PathBuf::from(SYSTEM_ROOT),
            run_root: PathBuf::from(RUN_ROOT),
            systemctl: PathBuf::from("systemctl"),
        }
    }

    #[cfg(test)]
    fn for_test(system_root: PathBuf, run_root: PathBuf, systemctl: PathBuf) -> Self {
        Self {
            system_root,
            run_root,
            systemctl,
        }
    }

    fn dropin_dir(&self, service: &str) -> PathBuf {
        self.system_root.join(format!("{service}.d"))
    }

    fn dropin_path(&self, service: &str) -> PathBuf {
        self.dropin_dir(service).join(DROPIN_NAME)
    }

    fn masks_dir(&self) -> PathBuf {
        self.run_root.join("masks")
    }

    fn owner_path(&self, service: &str) -> PathBuf {
        self.masks_dir().join(format!("{service}.owner"))
    }

    fn run_lock_path(&self, run_id: &str) -> PathBuf {
        self.run_root.join(format!("{run_id}.lock"))
    }
}

pub fn new_run_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{now}", std::process::id())
}

pub fn normalize_service(raw: &str) -> Result<String> {
    let service = raw
        .strip_suffix(".service")
        .unwrap_or(raw)
        .trim()
        .to_string();
    if service.is_empty() {
        bail!("--service must not be empty");
    }
    if service.contains('/') || service.contains("..") {
        bail!("--service must be a systemd service name, not a path");
    }
    if !service
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'@' | b':'))
    {
        bail!("--service contains unsupported characters");
    }
    Ok(format!("{service}.service"))
}

pub fn acquire_mask(service: &str, run_id: String) -> Result<MaskGuard> {
    acquire_mask_with_config(&SystemdMaskConfig::production(), service, run_id)
}

fn acquire_mask_with_config(
    config: &SystemdMaskConfig,
    service: &str,
    run_id: String,
) -> Result<MaskGuard> {
    let service = normalize_service(service)?;
    fs::create_dir_all(config.masks_dir()).context("create gild-storm mask lock dir")?;
    fs::create_dir_all(&config.run_root).context("create gild-storm run lock dir")?;
    let owner = MaskOwner {
        service: service.clone(),
        run_id: run_id.clone(),
    };
    let run_lock_path = config.run_lock_path(&run_id);
    let mut run_lock = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&run_lock_path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            bail!("gild-storm run lock already exists for run_id={run_id}")
        }
        Err(err) => return Err(err).with_context(|| format!("create {}", run_lock_path.display())),
    };
    writeln!(run_lock, "pid={}", std::process::id())
        .with_context(|| format!("write {}", run_lock_path.display()))?;

    let owner_path = config.owner_path(&service);
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&owner_path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&run_lock_path);
            bail!("{service} already has a gild-storm restart mask owner")
        }
        Err(err) => {
            let _ = fs::remove_file(&run_lock_path);
            return Err(err).with_context(|| format!("create {}", owner_path.display()));
        }
    };
    if let Err(err) = file
        .write_all(&serde_json::to_vec(&owner)?)
        .with_context(|| format!("write {}", owner_path.display()))
    {
        let _ = fs::remove_file(&run_lock_path);
        let _ = fs::remove_file(&owner_path);
        return Err(err);
    }
    audit(format!(
        "mask owner acquired service={service} run_id={run_id}"
    ));
    Ok(MaskGuard { service, run_id })
}

pub fn write_dropin(guard: &MaskGuard) -> Result<()> {
    write_dropin_with_config(&SystemdMaskConfig::production(), guard)
}

fn write_dropin_with_config(config: &SystemdMaskConfig, guard: &MaskGuard) -> Result<()> {
    fs::create_dir_all(config.dropin_dir(&guard.service))
        .with_context(|| format!("create drop-in dir for {}", guard.service))?;
    let path = config.dropin_path(&guard.service);
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            bail!("restart mask drop-in already exists for {}", guard.service)
        }
        Err(err) => return Err(err).with_context(|| format!("create {}", path.display())),
    };
    file.write_all(DROPIN_BODY)
        .with_context(|| format!("write {}", path.display()))?;
    audit(format!(
        "drop-in written service={} run_id={} path={}",
        guard.service,
        guard.run_id,
        path.display()
    ));
    Ok(())
}

pub fn remove_dropin(service: &str, run_id: &str) -> Result<()> {
    remove_dropin_with_config(&SystemdMaskConfig::production(), service, run_id)
}

fn remove_dropin_with_config(
    config: &SystemdMaskConfig,
    service: &str,
    run_id: &str,
) -> Result<()> {
    let service = normalize_service(service)?;
    let owner_path = config.owner_path(&service);
    if let Some(owner) = read_owner(&owner_path)?
        && owner.run_id != run_id
    {
        bail!(
            "refusing to remove restart mask for {service}: owned by run_id={}",
            owner.run_id
        );
    }

    let path = config.dropin_path(&service);
    match fs::remove_file(&path) {
        Ok(()) => audit(format!(
            "drop-in removed service={service} run_id={run_id} path={}",
            path.display()
        )),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            audit(format!(
                "drop-in already absent service={service} run_id={run_id} path={}",
                path.display()
            ));
        }
        Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
    }
    let _ = fs::remove_dir(config.dropin_dir(&service));
    match fs::remove_file(&owner_path) {
        Ok(()) => audit(format!(
            "mask owner removed service={service} run_id={run_id} path={}",
            owner_path.display()
        )),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("remove {}", owner_path.display())),
    }
    let run_lock_path = config.run_lock_path(run_id);
    match fs::remove_file(&run_lock_path) {
        Ok(()) => audit(format!(
            "run lock removed service={service} run_id={run_id} path={}",
            run_lock_path.display()
        )),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("remove {}", run_lock_path.display())),
    }
    Ok(())
}

pub fn daemon_reload() -> Result<()> {
    run_systemctl(&SystemdMaskConfig::production(), ["daemon-reload"])
}

pub fn ensure_started(service: &str) -> Result<()> {
    ensure_started_with_config(&SystemdMaskConfig::production(), service)
}

fn ensure_started_with_config(config: &SystemdMaskConfig, service: &str) -> Result<()> {
    let service = normalize_service(service)?;
    let _ = run_systemctl(config, ["reset-failed", service.as_str()]);
    run_systemctl(config, ["start", service.as_str()])?;
    audit(format!("service restarted service={service}"));
    Ok(())
}

pub fn signal_service(service: &str, signal: &str) -> Result<()> {
    let service = normalize_service(service)?;
    run_systemctl(
        &SystemdMaskConfig::production(),
        ["kill", "-s", signal, service.as_str()],
    )?;
    audit(format!(
        "service signaled service={service} signal={signal}"
    ));
    Ok(())
}

pub fn restore_masked_kill(service: &str, run_id: &str) -> Result<()> {
    restore_masked_kill_with_config(&SystemdMaskConfig::production(), service, run_id)
}

fn restore_masked_kill_with_config(
    config: &SystemdMaskConfig,
    service: &str,
    run_id: &str,
) -> Result<()> {
    let service = normalize_service(service)?;
    audit(format!(
        "restore-started service={service} run_id={run_id} mode=kill"
    ));
    remove_dropin_with_config(config, &service, run_id)?;
    run_systemctl(config, ["daemon-reload"])?;
    ensure_started_with_config(config, &service)?;
    audit(format!(
        "restore-complete service={service} run_id={run_id} mode=kill"
    ));
    Ok(())
}

pub fn restore_masked_pause(service: &str, run_id: &str) -> Result<()> {
    restore_masked_pause_with_config(&SystemdMaskConfig::production(), service, run_id)
}

fn restore_masked_pause_with_config(
    config: &SystemdMaskConfig,
    service: &str,
    run_id: &str,
) -> Result<()> {
    let service = normalize_service(service)?;
    audit(format!(
        "restore-started service={service} run_id={run_id} mode=pause"
    ));
    remove_dropin_with_config(config, &service, run_id)?;
    run_systemctl(config, ["daemon-reload"])?;
    run_systemctl(config, ["kill", "-s", "SIGCONT", service.as_str()])?;
    audit(format!("service signaled service={service} signal=SIGCONT"));
    ensure_started_with_config(config, &service)?;
    audit(format!(
        "restore-complete service={service} run_id={run_id} mode=pause"
    ));
    Ok(())
}

pub fn clear_storm_mask_if_present(service: &str) -> Result<()> {
    clear_storm_mask_if_present_with_config(&SystemdMaskConfig::production(), service)
}

fn clear_storm_mask_if_present_with_config(
    config: &SystemdMaskConfig,
    service: &str,
) -> Result<()> {
    let service = normalize_service(service)?;
    let owner_path = config.owner_path(&service);
    let run_id = read_owner(&owner_path)?
        .map(|owner| owner.run_id)
        .unwrap_or_else(|| "manual-clear".to_string());
    remove_dropin_with_config(config, &service, &run_id)?;
    run_systemctl(config, ["daemon-reload"])?;
    Ok(())
}

pub fn recover_orphaned_masks() -> Result<()> {
    recover_orphaned_masks_with_config(&SystemdMaskConfig::production())
}

fn recover_orphaned_masks_with_config(config: &SystemdMaskConfig) -> Result<()> {
    let masks_dir = config.masks_dir();
    if !masks_dir.exists() {
        audit("recover: no mask owner dir present");
        return Ok(());
    }
    for entry in
        fs::read_dir(&masks_dir).with_context(|| format!("read {}", masks_dir.display()))?
    {
        let entry = entry?;
        if entry.path().extension() != Some(OsStr::new("owner")) {
            continue;
        }
        let Some(owner) = read_owner(&entry.path())? else {
            continue;
        };
        audit(format!(
            "recover: clearing orphaned mask service={} run_id={}",
            owner.service, owner.run_id
        ));
        remove_dropin_with_config(config, &owner.service, &owner.run_id)?;
        run_systemctl(config, ["daemon-reload"])?;
        ensure_started_with_config(config, &owner.service)?;
    }
    Ok(())
}

fn read_owner(path: &Path) -> Result<Option<MaskOwner>> {
    match fs::read(path) {
        Ok(raw) => Ok(Some(
            serde_json::from_slice(&raw).with_context(|| format!("parse {}", path.display()))?,
        )),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

fn run_systemctl<const N: usize>(config: &SystemdMaskConfig, args: [&str; N]) -> Result<()> {
    let status = Command::new(&config.systemctl)
        .args(args)
        .status()
        .with_context(|| format!("run {}", config.systemctl.display()))?;
    if !status.success() {
        return Err(anyhow!(
            "{} {} exited with {status}",
            config.systemctl.display(),
            args.join(" ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_systemctl(dir: &Path, log: &Path) -> PathBuf {
        let bin = dir.join("systemctl");
        fs::write(
            &bin,
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", log.display()),
        )
        .unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();
        bin
    }

    #[test]
    fn service_names_are_normalized_and_restricted() {
        assert_eq!(
            normalize_service("gg.tana.gild-vault").unwrap(),
            "gg.tana.gild-vault.service"
        );
        assert_eq!(
            normalize_service("gg.tana.gild-vault.service").unwrap(),
            "gg.tana.gild-vault.service"
        );
        assert!(normalize_service("../vault").is_err());
        assert!(normalize_service("foo/bar").is_err());
    }

    #[test]
    fn mask_lifecycle_writes_dropin_and_rejects_concurrent_owner() {
        let temp = tempfile::tempdir().unwrap();
        let config = SystemdMaskConfig::for_test(
            temp.path().join("systemd"),
            temp.path().join("run"),
            PathBuf::from("true"),
        );
        let guard =
            acquire_mask_with_config(&config, "gg.tana.gild-vault", "run-a".to_string()).unwrap();
        write_dropin_with_config(&config, &guard).unwrap();
        assert!(config.run_lock_path(&guard.run_id).exists());
        assert_eq!(
            fs::read_to_string(config.dropin_path(&guard.service)).unwrap(),
            "[Service]\nRestart=no\n"
        );
        let err = acquire_mask_with_config(&config, "gg.tana.gild-vault", "run-b".to_string())
            .unwrap_err();
        assert!(err.to_string().contains("already has"));
        remove_dropin_with_config(&config, &guard.service, &guard.run_id).unwrap();
        assert!(!config.dropin_path(&guard.service).exists());
        assert!(!config.owner_path(&guard.service).exists());
        assert!(!config.run_lock_path(&guard.run_id).exists());
    }

    #[test]
    fn restore_masked_kill_removes_dropin_and_starts_service() {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("systemctl.log");
        let systemctl = fake_systemctl(temp.path(), &log);
        let config = SystemdMaskConfig::for_test(
            temp.path().join("systemd"),
            temp.path().join("run"),
            systemctl,
        );
        let guard =
            acquire_mask_with_config(&config, "gg.tana.gild-vault", "run-a".to_string()).unwrap();
        write_dropin_with_config(&config, &guard).unwrap();
        restore_masked_kill_with_config(&config, &guard.service, &guard.run_id).unwrap();

        assert!(!config.dropin_path(&guard.service).exists());
        let commands = fs::read_to_string(log).unwrap();
        assert!(commands.contains("daemon-reload"));
        assert!(commands.contains("reset-failed gg.tana.gild-vault.service"));
        assert!(commands.contains("start gg.tana.gild-vault.service"));
    }

    #[test]
    fn recover_clears_orphaned_mask() {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("systemctl.log");
        let systemctl = fake_systemctl(temp.path(), &log);
        let config = SystemdMaskConfig::for_test(
            temp.path().join("systemd"),
            temp.path().join("run"),
            systemctl,
        );
        let guard =
            acquire_mask_with_config(&config, "gg.tana.gild-vault", "run-a".to_string()).unwrap();
        write_dropin_with_config(&config, &guard).unwrap();

        recover_orphaned_masks_with_config(&config).unwrap();

        assert!(!config.dropin_path(&guard.service).exists());
        assert!(!config.owner_path(&guard.service).exists());
        let commands = fs::read_to_string(log).unwrap();
        assert!(commands.contains("daemon-reload"));
        assert!(commands.contains("start gg.tana.gild-vault.service"));
    }
}
