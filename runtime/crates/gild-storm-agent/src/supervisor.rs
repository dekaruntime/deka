use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::systemd_mask;

const LOG_PATH: &str = "/var/log/gild-storm-agent.log";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UndoAction {
    SystemctlStart { service: String },
    SystemctlStop { service: String },
    SignalService { service: String, signal: String },
    RestoreMaskedKill { service: String, run_id: String },
    RestoreMaskedPause { service: String, run_id: String },
    TailscaleUp,
    TcNetemDel { peer: String },
    RemoveFile { path: PathBuf },
    KillPid { pid: u32, signal: String },
    TouchFile { path: PathBuf },
    Noop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupervisorSpec {
    pub delay_ms: u64,
    pub action: UndoAction,
}

impl SupervisorSpec {
    pub fn new(delay: Duration, action: UndoAction) -> Self {
        Self {
            delay_ms: delay.as_millis().min(u128::from(u64::MAX)) as u64,
            action,
        }
    }

    fn delay(&self) -> Duration {
        Duration::from_millis(self.delay_ms)
    }
}

pub fn spawn_supervisor(delay: Duration, action: UndoAction) -> Result<PathBuf> {
    let spec = SupervisorSpec::new(delay, action);
    let spec_path = write_spec(&spec)?;
    detach_and_exec(&spec_path)?;
    audit(format!("spawned supervisor spec={}", spec_path.display()));
    Ok(spec_path)
}

pub fn run_supervisor(spec_path: &Path) -> Result<()> {
    let raw = fs::read_to_string(spec_path)
        .with_context(|| format!("read supervisor spec {}", spec_path.display()))?;
    let spec: SupervisorSpec = serde_json::from_str(&raw).context("parse supervisor spec")?;
    audit(format!(
        "supervisor armed delay_ms={} action={:?}",
        spec.delay_ms, spec.action
    ));
    thread::sleep(spec.delay());
    let result = run_action(&spec.action);
    match &result {
        Ok(()) => audit(format!("supervisor restored action={:?}", spec.action)),
        Err(err) => audit(format!(
            "supervisor restore failed action={:?}: {err}",
            spec.action
        )),
    }
    let _ = fs::remove_file(spec_path);
    result
}

pub fn run_action(action: &UndoAction) -> Result<()> {
    match action {
        UndoAction::SystemctlStart { service } => {
            run_status(Command::new("systemctl").arg("start").arg(service))
        }
        UndoAction::SystemctlStop { service } => {
            run_status(Command::new("systemctl").arg("stop").arg(service))
        }
        UndoAction::SignalService { service, signal } => run_status(
            Command::new("systemctl")
                .arg("kill")
                .arg("-s")
                .arg(signal)
                .arg(service),
        ),
        UndoAction::RestoreMaskedKill { service, run_id } => {
            systemd_mask::restore_masked_kill(service, run_id)
        }
        UndoAction::RestoreMaskedPause { service, run_id } => {
            systemd_mask::restore_masked_pause(service, run_id)
        }
        UndoAction::TailscaleUp => run_status(Command::new("tailscale").arg("up")),
        UndoAction::TcNetemDel { peer } => {
            run_status(Command::new("tc").args(["qdisc", "del", "dev", peer, "root", "netem"]))
        }
        UndoAction::RemoveFile { path } => {
            if path.exists() {
                fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
            }
            Ok(())
        }
        UndoAction::KillPid { pid, signal } => run_status(
            Command::new("kill")
                .arg(format!("-{signal}"))
                .arg(pid.to_string()),
        ),
        UndoAction::TouchFile { path } => {
            fs::write(path, b"restored\n").with_context(|| format!("touch {}", path.display()))
        }
        UndoAction::Noop => Ok(()),
    }
}

fn write_spec(spec: &SupervisorSpec) -> Result<PathBuf> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gild-storm-supervisor-{}-{now}.json",
        std::process::id()
    ));
    let json = serde_json::to_vec(spec)?;
    fs::write(&path, json).with_context(|| format!("write supervisor spec {}", path.display()))?;
    Ok(path)
}

fn detach_and_exec(spec_path: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("current executable")?;
    detach_and_exec_path(&exe, spec_path)
}

fn detach_and_exec_path(exe: &Path, spec_path: &Path) -> Result<()> {
    let exe_c = CString::new(exe.as_os_str().as_bytes())?;
    let arg0 = exe_c.clone();
    let arg1 = CString::new("__supervisor")?;
    let arg2 = CString::new("--spec")?;
    let arg3 = CString::new(spec_path.as_os_str().as_bytes())?;

    // SAFETY: fork/setsid/execvp are called with immutable C strings, and parents
    // exit or wait immediately. The grandchild does no Rust allocation before exec.
    unsafe {
        let first = libc::fork();
        if first < 0 {
            bail!("fork failed: {}", std::io::Error::last_os_error());
        }
        if first > 0 {
            let mut status = 0;
            libc::waitpid(first, &mut status, 0);
            return Ok(());
        }

        if libc::setsid() < 0 {
            libc::_exit(111);
        }
        let second = libc::fork();
        if second < 0 {
            libc::_exit(112);
        }
        if second > 0 {
            libc::_exit(0);
        }

        let devnull = CString::new("/dev/null").unwrap();
        let fd = libc::open(devnull.as_ptr(), libc::O_RDWR);
        if fd >= 0 {
            libc::dup2(fd, libc::STDIN_FILENO);
            libc::dup2(fd, libc::STDOUT_FILENO);
            libc::dup2(fd, libc::STDERR_FILENO);
            if fd > libc::STDERR_FILENO {
                libc::close(fd);
            }
        }

        let argv = [
            arg0.as_ptr(),
            arg1.as_ptr(),
            arg2.as_ptr(),
            arg3.as_ptr(),
            std::ptr::null(),
        ];
        libc::execv(exe_c.as_ptr(), argv.as_ptr());
        libc::_exit(127);
    }
}

fn run_status(cmd: &mut Command) -> Result<()> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd.status().with_context(|| format!("run {program}"))?;
    if !status.success() {
        return Err(anyhow!("{program} exited with {status}"));
    }
    Ok(())
}

pub fn audit(message: impl AsRef<str>) {
    let ts = humantime::format_rfc3339_seconds(SystemTime::now());
    let line = format!("{ts} {}\n", message.as_ref());
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o640)
        .open(LOG_PATH)
    {
        let _ = file.write_all(line.as_bytes());
    } else {
        eprint!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    #[test]
    fn supervisor_spec_round_trips() {
        let spec = SupervisorSpec::new(
            Duration::from_secs(2),
            UndoAction::RestoreMaskedKill {
                service: "gild-vault".to_string(),
                run_id: "run-a".to_string(),
            },
        );
        let json = serde_json::to_string(&spec).unwrap();
        let parsed: SupervisorSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, parsed);
    }

    #[test]
    fn detached_supervisor_outlives_spawner() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("marker");
        let spec = SupervisorSpec::new(
            Duration::from_millis(250),
            UndoAction::TouchFile {
                path: marker.clone(),
            },
        );
        let spec_path = write_spec(&spec).unwrap();
        let helper = temp.path().join("supervisor-helper");
        fs::write(
            &helper,
            r#"#!/bin/sh
spec="$3"
path=$(sed -n 's/.*"path":"\([^"]*\)".*/\1/p' "$spec")
sleep 0.25
printf 'restored\n' > "$path"
rm -f "$spec"
"#,
        )
        .unwrap();
        let mut perms = fs::metadata(&helper).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&helper, perms).unwrap();
        detach_and_exec_path(&helper, &spec_path).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if marker.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("supervisor did not create marker");
    }
}
