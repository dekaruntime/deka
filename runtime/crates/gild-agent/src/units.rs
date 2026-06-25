use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::audit::{chown_path, open_audit_log, sha256_hex, timestamp_millis, unique_tmp_path};
use crate::auth::PeerCredentials;
use crate::commands::validate_slug;
use crate::config::{
    AppState, BUN_BIN, DISPATCHER_HMAC_KEY_PLACEHOLDER, GILD_SOCKET_PATH, TANA_DIR,
};
use crate::protocol::ServiceReply;
use crate::requests::{DeleteUnitRequest, WriteUnitRequest};

pub(crate) fn handle_write_unit_request(
    state: &AppState,
    request: &WriteUnitRequest,
    peer: PeerCredentials,
) -> ServiceReply {
    match write_unit(
        request,
        &state.systemd_unit_root,
        &state.audit_log_path,
        peer,
        state.systemd_unit_owner,
    ) {
        Ok(reply) => ServiceReply::json_value(
            200,
            serde_json::json!({
                "ok": true,
                "path": reply.path,
                "sha256": reply.sha256
            }),
        ),
        Err(err) => {
            let status = if err
                .to_string()
                .contains("already exists with different sha256")
            {
                409
            } else {
                400
            };
            ServiceReply::json_value(status, serde_json::json!({ "error": err.to_string() }))
        }
    }
}

pub(crate) fn handle_delete_unit_request(
    state: &AppState,
    request: &DeleteUnitRequest,
    peer: PeerCredentials,
) -> ServiceReply {
    match delete_unit(
        request,
        &state.systemd_unit_root,
        &state.audit_log_path,
        peer,
    ) {
        Ok(reply) => ServiceReply::json_value(
            200,
            serde_json::json!({
                "ok": true,
                "path": reply.path,
                "existed": reply.existed
            }),
        ),
        Err(err) => ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() })),
    }
}

#[derive(Debug)]
pub(crate) struct WriteUnitReply {
    pub(crate) path: String,
    pub(crate) sha256: String,
}

#[derive(Debug)]
pub(crate) struct DeleteUnitReply {
    pub(crate) path: String,
    pub(crate) existed: bool,
}

pub(crate) fn write_unit(
    request: &WriteUnitRequest,
    unit_root: &Path,
    audit_path: &Path,
    peer: PeerCredentials,
    owner: Option<(u32, u32)>,
) -> Result<WriteUnitReply> {
    if request.op != "write_unit" {
        bail!("op must be write_unit");
    }
    validate_slug(&request.slug)?;
    validate_dispatcher_unit_kind(&request.kind)?;
    validate_env_map(&request.extra_env)?;

    let unit = dispatcher_unit_name(&request.slug);
    let rendered = render_dispatcher_unit(request);
    validate_unit_name(&unit)?;
    let path = unit_root.join(&unit);
    let contents = rendered.as_bytes();
    let sha256 = sha256_hex(contents);

    match fs::read(&path) {
        Ok(existing) => {
            let existing_sha256 = sha256_hex(&existing);
            if existing_sha256 != sha256 {
                bail!("unit already exists with different sha256");
            }
            write_unit_audit(audit_path, &unit, &sha256, peer)?;
            return Ok(WriteUnitReply {
                path: path.display().to_string(),
                sha256,
            });
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    }

    atomic_write_unit(&path, contents, owner)?;
    write_unit_audit(audit_path, &unit, &sha256, peer)?;
    Ok(WriteUnitReply {
        path: path.display().to_string(),
        sha256,
    })
}

pub(crate) fn validate_unit_name(unit: &str) -> Result<()> {
    static UNIT_RE: OnceLock<Regex> = OnceLock::new();
    let re = UNIT_RE.get_or_init(|| {
        Regex::new(r"^gg\.tana\.[a-z][a-z0-9.@-]+\.(service|timer)$").expect("unit regex compiles")
    });
    if re.is_match(unit) {
        Ok(())
    } else {
        bail!("invalid unit")
    }
}

pub(crate) fn dispatcher_unit_name(slug: &str) -> String {
    format!("gg.tana.gild-dispatcher@{slug}.service")
}

fn validate_dispatcher_unit_kind(kind: &str) -> Result<()> {
    if kind == "dispatcher" {
        Ok(())
    } else {
        bail!("kind must be dispatcher")
    }
}

fn validate_env_map(extra_env: &HashMap<String, String>) -> Result<()> {
    static ENV_NAME_RE: OnceLock<Regex> = OnceLock::new();
    let re = ENV_NAME_RE
        .get_or_init(|| Regex::new(r"^[A-Z][A-Z0-9_]{0,63}$").expect("env var regex compiles"));
    for (key, value) in extra_env {
        if !re.is_match(key) {
            bail!("invalid extra_env key {key:?}");
        }
        if value.contains('\n') || value.contains('\0') || value.len() > 512 {
            bail!("invalid extra_env value for {key}");
        }
    }
    Ok(())
}

pub(crate) fn render_dispatcher_unit(request: &WriteUnitRequest) -> String {
    let hmac_key = std::env::var("AGENT_DISPATCHER_HMAC_KEY")
        .unwrap_or_else(|_| DISPATCHER_HMAC_KEY_PLACEHOLDER.to_string());
    let mut env = request
        .extra_env
        .iter()
        .map(|(key, value)| format!("Environment={}={}", key, systemd_escape_env_value(value)))
        .collect::<Vec<_>>();
    env.sort();
    let extra_env = if env.is_empty() {
        String::new()
    } else {
        format!("{}\n", env.join("\n"))
    };
    format!(
        "[Unit]\n\
Description=Tana gild dispatcher for {slug}\n\
After=network-online.target\n\
Wants=network-online.target\n\
\n\
[Service]\n\
Type=simple\n\
User={slug}\n\
Group={slug}\n\
WorkingDirectory=/home/{slug}\n\
\n\
Environment=\"PATH=/home/{slug}/.local/bin:/home/{slug}/.bun/bin:/usr/local/bin:/usr/bin:/bin\"\n\
Environment=\"HOME=/home/{slug}\"\n\
Environment=\"AGENT_SLUG={slug}\"\n\
Environment=\"AGENT_DISPATCHER_PORT={port}\"\n\
Environment=\"AGENT_WORKSPACE=/home/{slug}/repos\"\n\
Environment=\"AGENT_DISPATCHER_HMAC_KEY={hmac_key}\"\n\
Environment=\"AGENT_PORTS_FILE={tana_dir}/infra/agent-ports.json\"\n\
Environment=\"AGENT_WORKER_SANDBOX=host\"\n\
Environment=\"GILD_SOCKET_PATH={gild_socket_path}\"\n\
Environment=\"GILD_BEARER_TOKEN_FILE=/home/{slug}/.config/tana/gild-bearer-token\"\n\
{extra_env}\
ExecStart={bun_bin} run {tana_dir}/agent-dispatcher/src/index.ts\n\
ExecStartPost=/bin/sh -c 'for attempt in 1 2 3 4 5 6 7 8 9 10; do /usr/bin/curl -fsS --max-time 2 http://127.0.0.1:${{AGENT_DISPATCHER_PORT}}/health && exit 0; sleep 1; done; exit 1'\n\
\n\
Restart=always\n\
RestartSec=3\n\
StandardOutput=append:/var/log/gg.tana.gild-dispatcher-{slug}.log\n\
StandardError=append:/var/log/gg.tana.gild-dispatcher-{slug}.log\n\
\n\
NoNewPrivileges=true\n\
ProtectSystem=strict\n\
ProtectHome=read-only\n\
ReadWritePaths=/home/{slug} /var/log /tmp\n\
PrivateTmp=true\n\
\n\
[Install]\n\
WantedBy=multi-user.target\n",
        slug = request.slug,
        port = request.port,
        hmac_key = hmac_key,
        tana_dir = TANA_DIR,
        gild_socket_path = GILD_SOCKET_PATH,
        bun_bin = BUN_BIN,
        extra_env = extra_env,
    )
}

fn systemd_escape_env_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(' ', "\\x20")
}

pub(crate) fn delete_unit(
    request: &DeleteUnitRequest,
    unit_root: &Path,
    audit_path: &Path,
    peer: PeerCredentials,
) -> Result<DeleteUnitReply> {
    if request.op != "delete_unit" {
        bail!("op must be delete_unit");
    }
    validate_unit_name(&request.unit)?;

    let path = unit_root.join(&request.unit);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(DeleteUnitReply {
                path: path.display().to_string(),
                existed: false,
            });
        }
        Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
    };

    if metadata.file_type().is_symlink() {
        refuse_symlink_escape(&path, unit_root)?;
    }

    let contents = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let sha256 = sha256_hex(&contents);
    fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    write_delete_unit_audit(audit_path, &request.unit, &sha256, peer)?;
    Ok(DeleteUnitReply {
        path: path.display().to_string(),
        existed: true,
    })
}

fn refuse_symlink_escape(path: &Path, unit_root: &Path) -> Result<()> {
    let root = fs::canonicalize(unit_root)
        .with_context(|| format!("canonicalize {}", unit_root.display()))?;
    let target =
        fs::canonicalize(path).with_context(|| format!("canonicalize {}", path.display()))?;
    if target.starts_with(&root) {
        Ok(())
    } else {
        bail!("refusing to delete symlink outside systemd unit root")
    }
}

pub(crate) fn atomic_write_unit(
    path: &Path,
    contents: &[u8],
    owner: Option<(u32, u32)>,
) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("unit path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let tmp_path = write_unique_unit_tmp(path, contents, owner)?;
    let result = match rename_noreplace(&tmp_path, path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path).with_context(|| format!("read {}", path.display()))?;
            if sha256_hex(&existing) != sha256_hex(contents) {
                Err(anyhow!("unit already exists with different sha256"))
            } else {
                fs::remove_file(&tmp_path)
                    .with_context(|| format!("remove {}", tmp_path.display()))?;
                Ok(())
            }
        }
        Err(err) => {
            Err(err).with_context(|| format!("rename {} to {}", tmp_path.display(), path.display()))
        }
    };

    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

fn write_unique_unit_tmp(
    path: &Path,
    contents: &[u8],
    owner: Option<(u32, u32)>,
) -> Result<PathBuf> {
    for _ in 0..16 {
        let tmp_path = unique_tmp_path(path);
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o644)
            .open(&tmp_path)
        {
            Ok(mut file) => {
                file.write_all(contents)
                    .with_context(|| format!("write {}", tmp_path.display()))?;
                file.sync_all()
                    .with_context(|| format!("sync {}", tmp_path.display()))?;
                fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o644))
                    .with_context(|| format!("chmod 0644 {}", tmp_path.display()))?;
                chown_path(&tmp_path, owner)?;
                return Ok(tmp_path);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err).with_context(|| format!("open {}", tmp_path.display())),
        }
    }

    bail!("could not allocate unique temporary unit path")
}

fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        match renameat2_noreplace(from, to) {
            Ok(()) => return Ok(()),
            Err(err)
                if err.raw_os_error() == Some(libc::ENOSYS)
                    || err.raw_os_error() == Some(libc::EINVAL) => {}
            Err(err) => return Err(err),
        }
    }

    fs::hard_link(from, to)?;
    fs::remove_file(from)
}

#[cfg(target_os = "linux")]
fn renameat2_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let from = CString::new(from.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let to = CString::new(to.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    let rc = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn write_unit_audit(
    path: &Path,
    unit: &str,
    sha256: &str,
    peer: PeerCredentials,
) -> Result<()> {
    let mut file = open_audit_log(path)?;
    let line = serde_json::json!({
        "ts": timestamp_millis().to_string(),
        "op": "write_unit",
        "unit": unit,
        "sha256": sha256,
        "by_uid": peer.uid,
        "by_pid": peer.pid
    });
    writeln!(file, "{line}").context("write audit log")
}

fn write_delete_unit_audit(
    path: &Path,
    unit: &str,
    sha256_removed: &str,
    peer: PeerCredentials,
) -> Result<()> {
    let mut file = open_audit_log(path)?;
    let line = serde_json::json!({
        "ts": timestamp_millis().to_string(),
        "op": "delete_unit",
        "unit": unit,
        "sha256_removed": sha256_removed,
        "existed": true,
        "by_uid": peer.uid,
        "by_pid": peer.pid
    });
    writeln!(file, "{line}").context("write audit log")
}
