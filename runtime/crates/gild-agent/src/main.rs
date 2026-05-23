use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

mod handlers;

const DEFAULT_SOCKET_PATH: &str = "/run/gild-agent.sock";
const ORCHESTRATOR_GROUP: &str = "gild-orchestrator";
const READ_TIMEOUT_MS: u64 = 1_000;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;
const DEFAULT_AUDIT_LOG_PATH: &str = "/var/log/gild-agent.log";
const PASSWD_PATH: &str = "/etc/passwd";
const GILD_AGENT_ROOT: &str = "/etc/gild/agents";
const DEFAULT_SYSTEMD_UNIT_ROOT: &str = "/etc/systemd/system";

#[derive(Clone)]
struct AppState {
    started_at: Instant,
    socket_path: PathBuf,
    orchestrator_gid: u32,
    audit_log_path: PathBuf,
    passwd_path: PathBuf,
    systemd_unit_root: PathBuf,
    systemd_unit_owner: Option<(u32, u32)>,
    command_runner: Arc<dyn CommandRunner>,
}

#[derive(Debug, Deserialize)]
struct CreateAgentRequest {
    slug: String,
    persona_ref: String,
}

#[derive(Debug, Deserialize)]
struct RemoveAgentRequest {
    slug: String,
}

#[derive(Debug, Deserialize)]
struct RestartUnitRequest {
    unit: String,
}

#[derive(Debug, Deserialize)]
struct SystemctlRequest {
    op: String,
    action: String,
    unit: String,
}

#[derive(Debug, Deserialize)]
struct RotateHmacRequest {
    op: String,
    slug: String,
}

#[derive(Debug, Deserialize)]
struct WriteUnitRequest {
    op: String,
    unit: String,
    contents: String,
}

#[derive(Debug, Serialize)]
struct OperationReply {
    ok: bool,
    operation: &'static str,
    message: String,
    exit: Option<i32>,
    stdout: String,
    stderr: String,
    error: Option<String>,
    pending: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct AuditRecord<'a> {
    ts: u64,
    op: &'a str,
    slug: &'a str,
    exit: Option<i32>,
    by_uid: u32,
    by_pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandOutput {
    success: bool,
    exit: Option<i32>,
    stdout: String,
    stderr: String,
}

struct CommandReplySpec<'a> {
    audit_op: &'static str,
    operation: &'static str,
    slug: &'a str,
    success_message: String,
    pending: Vec<&'static str>,
}

trait CommandRunner: Send + Sync {
    fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput>;
}

struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
        let output = Command::new(program).args(args).output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            exit: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
struct ServiceReply {
    status: u16,
    body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeerCredentials {
    pid: u32,
    uid: u32,
    gid: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    let state = Arc::new(load_state()?);
    serve(state).await
}

fn load_state() -> Result<AppState> {
    let group = std::env::var("GILD_AGENT_GROUP").unwrap_or_else(|_| ORCHESTRATOR_GROUP.into());
    let orchestrator_gid =
        group_gid(&group)?.ok_or_else(|| anyhow!("group {group:?} not found"))?;
    Ok(AppState {
        started_at: Instant::now(),
        socket_path: std::env::var("GILD_AGENT_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_SOCKET_PATH)),
        orchestrator_gid,
        audit_log_path: std::env::var("GILD_AGENT_AUDIT_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_AUDIT_LOG_PATH)),
        passwd_path: PathBuf::from(PASSWD_PATH),
        systemd_unit_root: std::env::var("GILD_AGENT_SYSTEMD_UNIT_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_SYSTEMD_UNIT_ROOT)),
        systemd_unit_owner: Some((0, 0)),
        command_runner: Arc::new(SystemCommandRunner),
    })
}

async fn serve(state: Arc<AppState>) -> Result<()> {
    prepare_socket_path(&state.socket_path)?;
    let listener = UnixListener::bind(&state.socket_path)
        .with_context(|| format!("bind {}", state.socket_path.display()))?;
    secure_socket(&state.socket_path, state.orchestrator_gid)?;
    eprintln!("gild-agent listening on {}", state.socket_path.display());

    loop {
        let (stream, _) = listener.accept().await.context("accept unix connection")?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, state).await {
                eprintln!("gild-agent connection error: {err:#}");
            }
        });
    }
}

async fn handle_connection(mut stream: UnixStream, state: Arc<AppState>) -> Result<()> {
    let peer = peer_credentials(&stream)?;
    if !authorize_peer(peer, state.orchestrator_gid)? {
        write_reply(
            &mut stream,
            ServiceReply::json_value(403, serde_json::json!({ "error": "forbidden" })),
        )
        .await?;
        return Ok(());
    }

    let request = timeout(
        Duration::from_millis(READ_TIMEOUT_MS),
        read_http_request(&mut stream),
    )
    .await
    .context("request read timed out")??;
    let reply = handle_request(&state, &request, peer).await;
    write_reply(&mut stream, reply).await
}

async fn handle_request(
    state: &AppState,
    request: &HttpRequest,
    peer: PeerCredentials,
) -> ServiceReply {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/v1/health") => ServiceReply::json_value(
            200,
            serde_json::json!({
                "ok": true,
                "uptime": state.started_at.elapsed().as_secs()
            }),
        ),
        ("POST", "/v1/agent/create") => match parse_json::<CreateAgentRequest>(request) {
            Ok(body) => create_agent(state, peer, &body),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/agent/remove") => match parse_json::<RemoveAgentRequest>(request) {
            Ok(body) => remove_agent(state, peer, &body),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/systemd/restart") => match parse_json::<RestartUnitRequest>(request) {
            Ok(body) => not_implemented("systemd.restart", &format!("unit {}", body.unit)),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/systemd/reload") => not_implemented("systemd.reload", "daemon-reload"),
        ("POST", "/v1/hmac/rotate") => match parse_json::<RotateHmacRequest>(request) {
            Ok(body) => handlers::hmac_rotate::handle_hmac_rotate_request(
                &body,
                peer,
                Path::new(GILD_AGENT_ROOT),
                &state.audit_log_path,
                Some((0, 0)),
            ),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/systemctl") => match parse_json::<SystemctlRequest>(request) {
            Ok(body) => handle_systemctl_request(state, &body, peer),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/write_unit") => match parse_json::<WriteUnitRequest>(request) {
            Ok(body) => handle_write_unit_request(state, &body, peer),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        (_, "/v1/health")
        | (_, "/v1/agent/create")
        | (_, "/v1/agent/remove")
        | (_, "/v1/systemd/restart")
        | (_, "/v1/systemd/reload")
        | (_, "/v1/hmac/rotate")
        | (_, "/v1/systemctl")
        | (_, "/v1/write_unit") => {
            ServiceReply::json_value(405, serde_json::json!({ "error": "method not allowed" }))
        }
        _ => ServiceReply::json_value(404, serde_json::json!({ "error": "not found" })),
    }
}

fn create_agent(
    state: &AppState,
    peer: PeerCredentials,
    request: &CreateAgentRequest,
) -> ServiceReply {
    if let Err(err) = validate_slug(&request.slug) {
        let _ = audit(state, "useradd", &request.slug, None, peer);
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }
    if request.persona_ref.trim().is_empty() {
        let _ = audit(state, "useradd", &request.slug, None, peer);
        return ServiceReply::json_value(
            400,
            serde_json::json!({ "error": "persona_ref is required" }),
        );
    }
    match user_exists(&request.slug, &state.passwd_path) {
        Ok(true) => {
            let _ = audit(state, "useradd", &request.slug, None, peer);
            return ServiceReply::json_value(
                409,
                serde_json::json!({ "error": format!("user {} already exists", request.slug) }),
            );
        }
        Ok(false) => {}
        Err(err) => {
            let _ = audit(state, "useradd", &request.slug, None, peer);
            return ServiceReply::json_value(500, serde_json::json!({ "error": err.to_string() }));
        }
    }

    let output = state.command_runner.output(
        "/usr/sbin/useradd",
        &[
            "--create-home",
            "--shell",
            "/bin/bash",
            "--gid",
            ORCHESTRATOR_GROUP,
            &request.slug,
        ],
    );
    command_reply(
        state,
        peer,
        output,
        CommandReplySpec {
            audit_op: "useradd",
            operation: "agent.create",
            slug: &request.slug,
            success_message: format!("created Linux user {}", request.slug),
            pending: vec!["systemd unit write", "sudoers setup"],
        },
    )
}

fn remove_agent(
    state: &AppState,
    peer: PeerCredentials,
    request: &RemoveAgentRequest,
) -> ServiceReply {
    if let Err(err) = validate_slug(&request.slug) {
        let _ = audit(state, "userdel", &request.slug, None, peer);
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }

    match running_processes(state, &request.slug) {
        Ok(Some(processes)) => {
            let _ = audit(state, "userdel", &request.slug, None, peer);
            return ServiceReply::json_value(
                409,
                serde_json::json!({
                    "ok": false,
                    "operation": "agent.remove",
                    "error": format!("user {} still has running processes", request.slug),
                    "processes": processes,
                }),
            );
        }
        Ok(None) => {}
        Err(err) => {
            let _ = audit(state, "userdel", &request.slug, None, peer);
            return ServiceReply::json_value(500, serde_json::json!({ "error": err.to_string() }));
        }
    }

    let output = state
        .command_runner
        .output("/usr/sbin/userdel", &["--remove", &request.slug]);
    command_reply(
        state,
        peer,
        output,
        CommandReplySpec {
            audit_op: "userdel",
            operation: "agent.remove",
            slug: &request.slug,
            success_message: format!("removed Linux user {}", request.slug),
            pending: vec!["systemd disable"],
        },
    )
}

fn handle_systemctl_request(
    state: &AppState,
    request: &SystemctlRequest,
    peer: PeerCredentials,
) -> ServiceReply {
    if request.op != "systemctl" {
        return ServiceReply::json_value(
            400,
            serde_json::json!({ "error": "op must be systemctl" }),
        );
    }
    if let Err(err) = validate_systemctl_action(&request.action) {
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }
    if let Err(err) = validate_systemctl_unit(&request.unit) {
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }

    let args = [request.action.as_str(), request.unit.as_str()];
    let output = match state.command_runner.output("/usr/bin/systemctl", &args) {
        Ok(result) => result,
        Err(err) => {
            let audit = write_systemctl_audit(
                &state.audit_log_path,
                &request.action,
                &request.unit,
                None,
                peer,
            );
            let audit_error = audit.err().map(|err| err.to_string());
            return ServiceReply::json_value(
                500,
                serde_json::json!({
                    "ok": false,
                    "op": "systemctl",
                    "action": request.action,
                    "unit": request.unit,
                    "exit": null,
                    "stdout": "",
                    "stderr": "",
                    "error": err.to_string(),
                    "audit_error": audit_error
                }),
            );
        }
    };

    let audit_error = write_systemctl_audit(
        &state.audit_log_path,
        &request.action,
        &request.unit,
        output.exit,
        peer,
    )
    .err()
    .map(|err| err.to_string());
    let ok = output.success;
    ServiceReply::json_value(
        if ok { 200 } else { 500 },
        serde_json::json!({
            "ok": ok,
            "op": "systemctl",
            "action": request.action,
            "unit": request.unit,
            "exit": output.exit,
            "stdout": output.stdout,
            "stderr": output.stderr,
            "audit_error": audit_error
        }),
    )
}

fn validate_systemctl_action(action: &str) -> Result<()> {
    match action {
        "start" | "stop" | "restart" | "reload" | "enable" | "disable" | "status" => Ok(()),
        _ => bail!("invalid systemctl action"),
    }
}

fn validate_systemctl_unit(unit: &str) -> Result<()> {
    static UNIT_RE: OnceLock<Regex> = OnceLock::new();
    let re = UNIT_RE.get_or_init(|| {
        Regex::new(r"^gg\.tana\.[a-z][a-z0-9.-]+\.(service|timer)$")
            .expect("systemctl unit regex compiles")
    });
    if re.is_match(unit) {
        Ok(())
    } else {
        bail!("invalid systemctl unit")
    }
}

fn write_systemctl_audit(
    path: &Path,
    action: &str,
    unit: &str,
    exit: Option<i32>,
    peer: PeerCredentials,
) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open audit log {}", path.display()))?;
    let line = serde_json::json!({
        "ts": timestamp_millis().to_string(),
        "op": "systemctl",
        "action": action,
        "unit": unit,
        "exit": exit,
        "by_uid": peer.uid,
        "by_pid": peer.pid
    });
    writeln!(file, "{line}").context("write audit log")
}

fn handle_write_unit_request(
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

#[derive(Debug)]
struct WriteUnitReply {
    path: String,
    sha256: String,
}

fn write_unit(
    request: &WriteUnitRequest,
    unit_root: &Path,
    audit_path: &Path,
    peer: PeerCredentials,
    owner: Option<(u32, u32)>,
) -> Result<WriteUnitReply> {
    if request.op != "write_unit" {
        bail!("op must be write_unit");
    }
    validate_write_unit_name(&request.unit)?;
    validate_unit_contents(&request.contents)?;

    let path = unit_root.join(&request.unit);
    let contents = request.contents.as_bytes();
    let sha256 = sha256_hex(contents);

    match fs::read(&path) {
        Ok(existing) => {
            let existing_sha256 = sha256_hex(&existing);
            if existing_sha256 != sha256 {
                bail!("unit already exists with different sha256");
            }
            write_unit_audit(audit_path, &request.unit, &sha256, peer)?;
            return Ok(WriteUnitReply {
                path: path.display().to_string(),
                sha256,
            });
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    }

    atomic_write_unit(&path, contents, owner)?;
    write_unit_audit(audit_path, &request.unit, &sha256, peer)?;
    Ok(WriteUnitReply {
        path: path.display().to_string(),
        sha256,
    })
}

fn validate_write_unit_name(unit: &str) -> Result<()> {
    static UNIT_RE: OnceLock<Regex> = OnceLock::new();
    let re = UNIT_RE.get_or_init(|| {
        Regex::new(r"^gg\.tana\.[a-z][a-z0-9.@-]+\.(service|timer)$")
            .expect("write_unit regex compiles")
    });
    if re.is_match(unit) {
        Ok(())
    } else {
        bail!("invalid unit")
    }
}

fn validate_unit_contents(contents: &str) -> Result<()> {
    if !has_unit_section(contents, "Unit") {
        bail!("unit contents must contain [Unit] section");
    }
    if has_unit_section(contents, "Service") || has_unit_section(contents, "Timer") {
        Ok(())
    } else {
        bail!("unit contents must contain [Service] or [Timer] section")
    }
}

fn has_unit_section(contents: &str, section: &str) -> bool {
    let expected = format!("[{section}]");
    contents.lines().any(|line| line.trim() == expected)
}

fn atomic_write_unit(path: &Path, contents: &[u8], owner: Option<(u32, u32)>) -> Result<()> {
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
                fs::rename(&tmp_path, path)
                    .with_context(|| format!("rename {} to {}", tmp_path.display(), path.display()))
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

fn write_unit_audit(path: &Path, unit: &str, sha256: &str, peer: PeerCredentials) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open audit log {}", path.display()))?;
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

fn chown_path(path: &Path, owner: Option<(u32, u32)>) -> Result<()> {
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

fn unique_tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(format!(
        ".tmp.{}.{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    PathBuf::from(tmp)
}

fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    hex_encode(&digest)
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

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn not_implemented(operation: &str, target: &str) -> ServiceReply {
    ServiceReply::json_value(
        501,
        serde_json::json!({
            "ok": false,
            "operation": operation,
            "target": target,
            "error": "not yet implemented"
        }),
    )
}

fn parse_json<T: for<'de> Deserialize<'de>>(request: &HttpRequest) -> Result<T> {
    if request.body.is_empty() {
        bail!("request body is required");
    }
    serde_json::from_slice(&request.body).context("parse JSON body")
}

fn validate_slug(slug: &str) -> Result<()> {
    static SLUG_RE: OnceLock<Regex> = OnceLock::new();
    let re = SLUG_RE.get_or_init(|| {
        Regex::new(r"^agent-[a-z][a-z0-9-]{1,30}$").expect("agent slug regex compiles")
    });
    if !re.is_match(slug) {
        bail!("slug must match ^agent-[a-z][a-z0-9-]{{1,30}}$");
    }
    Ok(())
}

fn command_reply(
    state: &AppState,
    peer: PeerCredentials,
    output: io::Result<CommandOutput>,
    spec: CommandReplySpec<'_>,
) -> ServiceReply {
    match output {
        Ok(output) => {
            let _ = audit(state, spec.audit_op, spec.slug, output.exit, peer);
            let error = (!output.success).then(|| {
                format!(
                    "{} failed with exit {:?}: {}",
                    spec.audit_op,
                    output.exit,
                    output.stderr.trim()
                )
            });
            let status = if output.success { 200 } else { 500 };
            ServiceReply::json_value(
                status,
                serde_json::json!(OperationReply {
                    ok: output.success,
                    operation: spec.operation,
                    message: if output.success {
                        spec.success_message
                    } else {
                        format!("{} failed for {}", spec.audit_op, spec.slug)
                    },
                    exit: output.exit,
                    stdout: output.stdout,
                    stderr: output.stderr,
                    error,
                    pending: spec.pending,
                }),
            )
        }
        Err(err) => {
            let _ = audit(state, spec.audit_op, spec.slug, None, peer);
            ServiceReply::json_value(
                500,
                serde_json::json!(OperationReply {
                    ok: false,
                    operation: spec.operation,
                    message: format!("{} failed for {}", spec.audit_op, spec.slug),
                    exit: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    error: Some(err.to_string()),
                    pending: spec.pending,
                }),
            )
        }
    }
}

fn user_exists(username: &str, passwd_path: &Path) -> Result<bool> {
    let raw = match fs::read_to_string(passwd_path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("read {}", passwd_path.display())),
    };
    Ok(raw.lines().any(|line| {
        line.split_once(':')
            .map(|(name, _)| name == username)
            .unwrap_or(false)
    }))
}

fn running_processes(state: &AppState, username: &str) -> Result<Option<Vec<u32>>> {
    let output = state
        .command_runner
        .output("/usr/bin/pgrep", &["-u", username])
        .context("run /usr/bin/pgrep")?;
    match output.exit {
        Some(0) => {
            let processes = output
                .stdout
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect::<Vec<_>>();
            Ok(Some(processes))
        }
        Some(1) => Ok(None),
        _ => bail!(
            "pgrep failed with exit {:?}: {}",
            output.exit,
            output.stderr.trim()
        ),
    }
}

fn audit(
    state: &AppState,
    op: &'static str,
    slug: &str,
    exit: Option<i32>,
    peer: PeerCredentials,
) -> Result<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before UNIX epoch")?
        .as_secs();
    let line = serde_json::to_string(&AuditRecord {
        ts,
        op,
        slug,
        exit,
        by_uid: peer.uid,
        by_pid: peer.pid,
    })?;
    if let Some(parent) = state.audit_log_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    use std::io::Write;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.audit_log_path)
        .with_context(|| format!("open audit log {}", state.audit_log_path.display()))?;
    writeln!(file, "{line}").context("write audit log")?;
    eprintln!("{line}");
    Ok(())
}

async fn read_http_request(stream: &mut UnixStream) -> Result<HttpRequest> {
    let mut buf = Vec::with_capacity(4096);
    let header_end = loop {
        if buf.len() >= MAX_HEADER_BYTES {
            bail!("request headers too large");
        }
        let mut chunk = [0_u8; 1024];
        let n = stream.read(&mut chunk).await.context("read request")?;
        if n == 0 {
            bail!("client closed before request headers");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
    };

    let raw_headers = std::str::from_utf8(&buf[..header_end]).context("headers are not UTF-8")?;
    let (method, path, headers) = parse_http_head(raw_headers)?;
    let content_length = content_length(&headers)?;
    if content_length > MAX_BODY_BYTES {
        bail!("request body too large");
    }

    let body_start = header_end + 4;
    let mut body = buf[body_start..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let mut chunk = vec![0_u8; remaining.min(4096)];
        let n = stream.read(&mut chunk).await.context("read request body")?;
        if n == 0 {
            bail!("client closed before request body");
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

#[cfg(test)]
fn parse_http_request(raw: &str) -> Result<HttpRequest> {
    let header_end = raw
        .find("\r\n\r\n")
        .ok_or_else(|| anyhow!("missing header terminator"))?;
    let (method, path, headers) = parse_http_head(&raw[..header_end])?;
    let body = raw.as_bytes()[header_end + 4..].to_vec();
    let content_length = content_length(&headers)?;
    if body.len() < content_length {
        bail!("request body shorter than content-length");
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: body[..content_length].to_vec(),
    })
}

fn parse_http_head(raw: &str) -> Result<(String, String, HashMap<String, String>)> {
    let mut lines = raw.lines();
    let request_line = lines.next().ok_or_else(|| anyhow!("empty HTTP request"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP method"))?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP path"))?
        .to_string();
    let version = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP version"))?;
    if !version.starts_with("HTTP/") {
        bail!("invalid HTTP version");
    }

    let mut headers = HashMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            bail!("malformed HTTP header");
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    Ok((method, path, headers))
}

fn content_length(headers: &HashMap<String, String>) -> Result<usize> {
    headers
        .get("content-length")
        .map(|value| value.parse::<usize>().context("invalid content-length"))
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn http_response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        _ => "OK",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\n\r\n{body}",
        body.len()
    )
}

async fn write_reply(stream: &mut UnixStream, reply: ServiceReply) -> Result<()> {
    stream
        .write_all(http_response(reply.status, &reply.body).as_bytes())
        .await
        .context("write response")
}

impl ServiceReply {
    fn json_value(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            body: value.to_string(),
        }
    }
}

fn peer_credentials(stream: &UnixStream) -> Result<PeerCredentials> {
    let fd = stream.as_raw_fd();
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error()).context("getsockopt SO_PEERCRED");
    }
    peer_credentials_from_ucred(cred)
}

fn peer_credentials_from_ucred(cred: libc::ucred) -> Result<PeerCredentials> {
    Ok(PeerCredentials {
        pid: u32::try_from(cred.pid).context("peer pid is negative")?,
        uid: cred.uid,
        gid: cred.gid,
    })
}

fn authorize_peer(peer: PeerCredentials, orchestrator_gid: u32) -> Result<bool> {
    if peer.uid == 0 || peer.gid == orchestrator_gid {
        return Ok(true);
    }
    Ok(user_group_ids(peer.uid)?.contains(&orchestrator_gid))
}

fn group_gid(group: &str) -> Result<Option<u32>> {
    let raw = match fs::read_to_string("/etc/group") {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("read /etc/group"),
    };
    Ok(raw.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        let gid = fields.next()?;
        (name == group).then(|| gid.parse::<u32>().ok()).flatten()
    }))
}

fn user_group_ids(uid: u32) -> Result<Vec<u32>> {
    let username = username_for_uid(uid)?;
    let Some(username) = username else {
        return Ok(Vec::new());
    };
    let raw = fs::read_to_string("/etc/group").context("read /etc/group")?;
    Ok(raw
        .lines()
        .filter_map(|line| group_line_contains_user(line, &username))
        .collect())
}

fn username_for_uid(uid: u32) -> Result<Option<String>> {
    let raw = match fs::read_to_string("/etc/passwd") {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("read /etc/passwd"),
    };
    Ok(raw.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        let parsed_uid = fields.next()?.parse::<u32>().ok()?;
        (parsed_uid == uid).then(|| name.to_string())
    }))
}

fn group_line_contains_user(line: &str, username: &str) -> Option<u32> {
    let mut fields = line.split(':');
    let _name = fields.next()?;
    let _password = fields.next()?;
    let gid = fields.next()?.parse::<u32>().ok()?;
    let members = fields.next().unwrap_or_default();
    members
        .split(',')
        .any(|member| member == username)
        .then_some(gid)
}

fn prepare_socket_path(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path)
                .with_context(|| format!("remove stale socket {}", path.display()))?;
        }
        Ok(_) => bail!("{} exists and is not a socket", path.display()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
    }
    Ok(())
}

fn secure_socket(path: &Path, gid: u32) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))
        .with_context(|| format!("chmod 0660 {}", path.display()))?;
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .context("socket path contains NUL")?;
    let rc = unsafe { libc::chown(c_path.as_ptr(), 0, gid) };
    if rc != 0 {
        return Err(io::Error::last_os_error())
            .with_context(|| format!("chown root:{gid} {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Barrier, Mutex};
    use std::time::SystemTime;
    use tokio::net::UnixStream;

    struct FakeRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        outputs: Mutex<VecDeque<io::Result<CommandOutput>>>,
    }

    impl FakeRunner {
        fn new(outputs: Vec<io::Result<CommandOutput>>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                outputs: Mutex::new(outputs.into()),
            })
        }

        fn success() -> CommandOutput {
            CommandOutput {
                success: true,
                exit: Some(0),
                stdout: "ok\n".to_string(),
                stderr: String::new(),
            }
        }

        fn exit(exit: i32, stdout: &str, stderr: &str) -> CommandOutput {
            CommandOutput {
                success: exit == 0,
                exit: Some(exit),
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
            self.calls.lock().unwrap().push((
                program.to_string(),
                args.iter().map(|arg| arg.to_string()).collect(),
            ));
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(Self::success()))
        }
    }

    fn test_state(command_runner: Arc<dyn CommandRunner>) -> AppState {
        let dir = std::env::temp_dir().join(format!(
            "gild-agent-test-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let passwd_path = dir.join("passwd");
        fs::write(&passwd_path, "root:x:0:0:root:/root:/bin/bash\n").unwrap();
        AppState {
            started_at: Instant::now(),
            socket_path: dir.join("gild-agent-test.sock"),
            orchestrator_gid: 1,
            audit_log_path: dir.join("gild-agent.log"),
            passwd_path,
            systemd_unit_root: dir.join("systemd"),
            systemd_unit_owner: None,
            command_runner,
        }
    }

    fn test_peer() -> PeerCredentials {
        PeerCredentials {
            pid: 4242,
            uid: 1001,
            gid: 1001,
        }
    }

    #[test]
    fn parses_post_request_with_json_body() {
        let body = "{\"slug\":\"agent-khalid\",\"persona_ref\":\"abc\"}";
        let raw = format!(
            "POST /v1/agent/create HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {body}",
            body.len()
        );
        let request = parse_http_request(&raw).unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/agent/create");
        assert_eq!(
            request.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        let body: CreateAgentRequest = parse_json(&request).unwrap();
        assert_eq!(body.slug, "agent-khalid");
        assert_eq!(body.persona_ref, "abc");
    }

    #[test]
    fn rejects_short_body() {
        let err = parse_http_request(
            "POST /v1/agent/remove HTTP/1.1\r\nContent-Length: 99\r\n\r\n{\"slug\":\"a\"}",
        )
        .unwrap_err();
        assert!(err.to_string().contains("shorter than content-length"));
    }

    #[tokio::test]
    async fn health_response_shape() {
        let state = AppState {
            started_at: Instant::now(),
            socket_path: PathBuf::from("/tmp/gild-agent-test.sock"),
            orchestrator_gid: 1,
            audit_log_path: PathBuf::from("/tmp/gild-agent-test.log"),
            passwd_path: PathBuf::from("/tmp/gild-agent-test-passwd"),
            systemd_unit_root: std::env::temp_dir().join("gild-agent-test-systemd"),
            systemd_unit_owner: None,
            command_runner: Arc::new(SystemCommandRunner),
        };
        let request =
            parse_http_request("GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let reply = handle_request(&state, &request, test_peer()).await;
        assert_eq!(reply.status, 200);
        let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(json["ok"], true);
        assert!(json["uptime"].as_u64().is_some());
    }

    #[test]
    fn response_shape_includes_content_length() {
        let response = http_response(501, "{\"error\":\"not yet implemented\"}");
        assert!(response.starts_with("HTTP/1.1 501 Not Implemented"));
        assert!(response.contains("content-type: application/json"));
        assert!(response.ends_with("{\"error\":\"not yet implemented\"}"));
    }

    #[test]
    fn extracts_peercred_from_raw_ucred() {
        let cred = libc::ucred {
            pid: 123,
            uid: 456,
            gid: 789,
        };
        assert_eq!(
            peer_credentials_from_ucred(cred).unwrap(),
            PeerCredentials {
                pid: 123,
                uid: 456,
                gid: 789,
            }
        );
    }

    #[test]
    fn rejects_negative_peer_pid() {
        let cred = libc::ucred {
            pid: -1,
            uid: 456,
            gid: 789,
        };
        assert!(peer_credentials_from_ucred(cred).is_err());
    }

    #[tokio::test]
    async fn extracts_peercred_from_unix_socket() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let cred = peer_credentials(&stream).unwrap();
        assert_eq!(cred.pid, std::process::id());
        assert_eq!(cred.uid, unsafe { libc::geteuid() });
        assert_eq!(cred.gid, unsafe { libc::getegid() });
    }

    #[test]
    fn group_line_membership_parser() {
        assert_eq!(
            group_line_contains_user("gild-orchestrator:x:777:ava,agent-khalid", "agent-khalid"),
            Some(777)
        );
        assert_eq!(
            group_line_contains_user("gild-orchestrator:x:777:ava", "agent-khalid"),
            None
        );
    }

    #[test]
    fn validates_slug() {
        assert!(validate_slug("agent-khalid").is_ok());
        assert!(validate_slug("agent-zed").is_ok());
        assert!(validate_slug("agent-a1").is_ok());
        assert!(validate_slug("agent-a-b").is_ok());
        assert!(validate_slug("agent-khalid-123456789012345678").is_ok());
        assert!(validate_slug("khalid").is_err());
        assert!(validate_slug("agent-a").is_err());
        assert!(validate_slug("AgentKhalid").is_err());
        assert!(validate_slug("agent_khalid").is_err());
        assert!(validate_slug("agent-1a").is_err());
        assert!(validate_slug("agent-a_b").is_err());
        assert!(validate_slug("agent-khalid-1234567890123456789012345").is_err());
    }

    #[tokio::test]
    async fn create_agent_invokes_useradd_and_audits_peercred() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::success())]);
        let state = test_state(runner.clone());
        let request = parse_http_request(
            "POST /v1/agent/create HTTP/1.1\r\nContent-Length: 49\r\n\r\n{\"slug\":\"agent-zed\",\"persona_ref\":\"personas/zed\"}",
        )
        .unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 200);
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [(
                "/usr/sbin/useradd".to_string(),
                vec![
                    "--create-home".to_string(),
                    "--shell".to_string(),
                    "/bin/bash".to_string(),
                    "--gid".to_string(),
                    "gild-orchestrator".to_string(),
                    "agent-zed".to_string(),
                ],
            )]
        );
        let audit = fs::read_to_string(&state.audit_log_path).unwrap();
        let line: serde_json::Value = serde_json::from_str(audit.trim()).unwrap();
        assert_eq!(line["op"], "useradd");
        assert_eq!(line["slug"], "agent-zed");
        assert_eq!(line["exit"], 0);
        assert_eq!(line["by_uid"], 1001);
        assert_eq!(line["by_pid"], 4242);
        assert!(line["ts"].as_u64().is_some());
    }

    #[tokio::test]
    async fn create_agent_refuses_existing_user_without_useradd() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::success())]);
        let state = test_state(runner.clone());
        fs::write(
            &state.passwd_path,
            "root:x:0:0:root:/root:/bin/bash\nagent-zed:x:1002:1002::/home/agent-zed:/bin/bash\n",
        )
        .unwrap();
        let request = parse_http_request(
            "POST /v1/agent/create HTTP/1.1\r\nContent-Length: 49\r\n\r\n{\"slug\":\"agent-zed\",\"persona_ref\":\"personas/zed\"}",
        )
        .unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 409);
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn remove_agent_refuses_running_processes() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::exit(0, "123\n456\n", ""))]);
        let state = test_state(runner.clone());
        let request = parse_http_request(
            "POST /v1/agent/remove HTTP/1.1\r\nContent-Length: 20\r\n\r\n{\"slug\":\"agent-zed\"}",
        )
        .unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 409);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "/usr/bin/pgrep");
        assert_eq!(calls[0].1, vec!["-u".to_string(), "agent-zed".to_string()]);
        assert!(reply.body.contains("running processes"));
    }

    #[tokio::test]
    async fn remove_agent_invokes_userdel_after_empty_pgrep() {
        let runner = FakeRunner::new(vec![
            Ok(FakeRunner::exit(1, "", "")),
            Ok(FakeRunner::success()),
        ]);
        let state = test_state(runner.clone());
        let request = parse_http_request(
            "POST /v1/agent/remove HTTP/1.1\r\nContent-Length: 20\r\n\r\n{\"slug\":\"agent-zed\"}",
        )
        .unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 200);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[1].0, "/usr/sbin/userdel");
        assert_eq!(
            calls[1].1,
            vec!["--remove".to_string(), "agent-zed".to_string()]
        );
    }

    #[test]
    fn validates_systemctl_action_whitelist() {
        for action in [
            "start", "stop", "restart", "reload", "enable", "disable", "status",
        ] {
            assert!(validate_systemctl_action(action).is_ok(), "{action}");
        }

        assert!(validate_systemctl_action("daemon-reload").is_err());
        assert!(validate_systemctl_action("reboot").is_err());
        assert!(validate_systemctl_action("").is_err());
    }

    #[test]
    fn validates_systemctl_unit_pattern() {
        assert!(validate_systemctl_unit("gg.tana.agent-foo.service").is_ok());
        assert!(validate_systemctl_unit("gg.tana.agent.foo-1.timer").is_ok());

        assert!(validate_systemctl_unit("nginx.service").is_err());
        assert!(validate_systemctl_unit("../../etc/passwd").is_err());
        assert!(validate_systemctl_unit("gg.tana.AgentFoo.service").is_err());
        assert!(validate_systemctl_unit("gg.tana.agent-foo.socket").is_err());
    }

    #[test]
    fn systemctl_handler_invokes_runner_and_audits() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::exit(0, "started\n", ""))]);
        let state = test_state(runner.clone());
        let request = SystemctlRequest {
            op: "systemctl".to_string(),
            action: "start".to_string(),
            unit: "gg.tana.agent-foo.service".to_string(),
        };

        let reply = handle_systemctl_request(&state, &request, test_peer());

        assert_eq!(reply.status, 200);
        let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["exit"], 0);
        assert_eq!(json["stdout"], "started\n");
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [(
                "/usr/bin/systemctl".to_string(),
                vec!["start".to_string(), "gg.tana.agent-foo.service".to_string()]
            )]
        );

        let audit = fs::read_to_string(&state.audit_log_path).unwrap();
        let audit_json: serde_json::Value = serde_json::from_str(audit.trim()).unwrap();
        assert_eq!(audit_json["op"], "systemctl");
        assert_eq!(audit_json["action"], "start");
        assert_eq!(audit_json["unit"], "gg.tana.agent-foo.service");
        assert_eq!(audit_json["exit"], 0);
        assert_eq!(audit_json["by_uid"], 1001);
        assert_eq!(audit_json["by_pid"], 4242);
    }

    #[test]
    fn systemctl_handler_rejects_invalid_request_before_runner() {
        let runner = FakeRunner::new(vec![Ok(FakeRunner::success())]);
        let state = test_state(runner.clone());
        let request = SystemctlRequest {
            op: "systemctl".to_string(),
            action: "restart".to_string(),
            unit: "nginx.service".to_string(),
        };

        let reply = handle_systemctl_request(&state, &request, test_peer());

        assert_eq!(reply.status, 400);
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn validates_write_unit_name_pattern() {
        assert!(validate_write_unit_name("gg.tana.gild-dispatcher@agent-foo.service").is_ok());
        assert!(validate_write_unit_name("gg.tana.agent.foo-1.timer").is_ok());

        assert!(validate_write_unit_name("nginx.service").is_err());
        assert!(validate_write_unit_name("../../etc/passwd").is_err());
        assert!(validate_write_unit_name("gg.tana.AgentFoo.service").is_err());
        assert!(validate_write_unit_name("gg.tana.1agent.service").is_err());
        assert!(validate_write_unit_name("gg.tana.agent_foo.service").is_err());
        assert!(validate_write_unit_name("gg.tana.agent-foo.socket").is_err());
    }

    #[test]
    fn validates_unit_contents_shape() {
        assert!(
            validate_unit_contents("[Unit]\nDescription=x\n[Service]\nExecStart=/bin/true\n")
                .is_ok()
        );
        assert!(validate_unit_contents("[Unit]\nDescription=x\n[Timer]\nOnBootSec=1m\n").is_ok());

        assert!(validate_unit_contents("[Service]\nExecStart=/bin/true\n").is_err());
        assert!(validate_unit_contents("[Unit]\nDescription=x\n").is_err());
        assert!(validate_unit_contents("not a unit").is_err());
    }

    #[test]
    fn write_unit_is_idempotent_but_rejects_different_existing_contents() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let audit = state.audit_log_path.clone();
        let request = WriteUnitRequest {
            op: "write_unit".to_string(),
            unit: "gg.tana.gild-dispatcher@agent-foo.service".to_string(),
            contents: "[Unit]\nDescription=agent foo\n[Service]\nExecStart=/bin/true\n".to_string(),
        };

        let first = write_unit(
            &request,
            &state.systemd_unit_root,
            &audit,
            test_peer(),
            None,
        )
        .unwrap();
        let second = write_unit(
            &request,
            &state.systemd_unit_root,
            &audit,
            test_peer(),
            None,
        )
        .unwrap();

        assert_eq!(first.sha256, second.sha256);
        let path = state.systemd_unit_root.join(&request.unit);
        assert_eq!(first.path, path.display().to_string());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert!(!path.with_extension("service.tmp").exists());

        let changed = WriteUnitRequest {
            contents: "[Unit]\nDescription=changed\n[Service]\nExecStart=/bin/true\n".to_string(),
            ..request
        };
        let err = write_unit(
            &changed,
            &state.systemd_unit_root,
            &audit,
            test_peer(),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("different sha256"));

        let audit_lines = fs::read_to_string(&audit).unwrap();
        assert_eq!(audit_lines.lines().count(), 2);
        let audit_json: serde_json::Value =
            serde_json::from_str(audit_lines.lines().next().unwrap()).unwrap();
        assert_eq!(audit_json["op"], "write_unit");
        assert_eq!(
            audit_json["unit"],
            "gg.tana.gild-dispatcher@agent-foo.service"
        );
        assert_eq!(audit_json["sha256"], first.sha256);
        assert_eq!(audit_json["by_uid"], 1001);
        assert_eq!(audit_json["by_pid"], 4242);
    }

    #[test]
    fn concurrent_write_unit_same_unit_conflicts_without_tmp_orphans() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let unit = "gg.tana.gild-dispatcher@agent-race.service".to_string();
        let first_contents =
            "[Unit]\nDescription=agent race one\n[Service]\nExecStart=/bin/true\n".to_string();
        let second_contents =
            "[Unit]\nDescription=agent race two\n[Service]\nExecStart=/bin/true\n".to_string();
        let barrier = Arc::new(Barrier::new(3));

        let handles = [first_contents.clone(), second_contents.clone()].map(|contents| {
            let state = state.clone();
            let unit = unit.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let request = WriteUnitRequest {
                    op: "write_unit".to_string(),
                    unit,
                    contents,
                };
                barrier.wait();
                handle_write_unit_request(&state, &request, test_peer())
            })
        });

        barrier.wait();
        let replies: Vec<ServiceReply> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        let mut statuses: Vec<u16> = replies.iter().map(|reply| reply.status).collect();
        statuses.sort_unstable();
        assert_eq!(statuses, vec![200, 409]);

        let target = state.systemd_unit_root.join(&unit);
        let written = fs::read_to_string(&target).unwrap();
        assert!(written == first_contents || written == second_contents);

        let prefix = format!("{unit}.tmp.");
        let leftovers: Vec<PathBuf> = fs::read_dir(&state.systemd_unit_root)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name()?.to_str()?;
                name.starts_with(&prefix).then_some(path)
            })
            .collect();
        assert!(leftovers.is_empty(), "leftover tmp files: {leftovers:?}");
    }

    #[tokio::test]
    async fn write_unit_route_writes_to_configured_root() {
        let runner = FakeRunner::new(vec![]);
        let state = test_state(runner);
        let body = serde_json::json!({
            "op": "write_unit",
            "unit": "gg.tana.gild-dispatcher@agent-bar.service",
            "contents": "[Unit]\nDescription=agent bar\n[Service]\nExecStart=/bin/true\n"
        })
        .to_string();
        let raw = format!(
            "POST /v1/write_unit HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let request = parse_http_request(&raw).unwrap();

        let reply = handle_request(&state, &request, test_peer()).await;

        assert_eq!(reply.status, 200);
        let json: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(
            json["path"],
            state
                .systemd_unit_root
                .join("gg.tana.gild-dispatcher@agent-bar.service")
                .display()
                .to_string()
        );
        assert!(json["sha256"].as_str().unwrap().len() == 64);
    }
}
