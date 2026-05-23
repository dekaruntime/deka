use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

const DEFAULT_SOCKET_PATH: &str = "/run/gild-agent.sock";
const ORCHESTRATOR_GROUP: &str = "gild-orchestrator";
const READ_TIMEOUT_MS: u64 = 1_000;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct AppState {
    started_at: Instant,
    socket_path: PathBuf,
    orchestrator_gid: u32,
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
struct RotateHmacRequest {
    agent_slug: String,
}

#[derive(Debug, Serialize)]
struct OperationReply<'a> {
    ok: bool,
    operation: &'a str,
    message: String,
    pending: Vec<&'a str>,
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
    let reply = handle_request(&state, &request).await;
    write_reply(&mut stream, reply).await
}

async fn handle_request(state: &AppState, request: &HttpRequest) -> ServiceReply {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/v1/health") => ServiceReply::json_value(
            200,
            serde_json::json!({
                "ok": true,
                "uptime": state.started_at.elapsed().as_secs()
            }),
        ),
        ("POST", "/v1/agent/create") => match parse_json::<CreateAgentRequest>(request) {
            Ok(body) => create_agent(&body),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/agent/remove") => match parse_json::<RemoveAgentRequest>(request) {
            Ok(body) => remove_agent(&body),
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
            Ok(body) => not_implemented("hmac.rotate", &format!("agent {}", body.agent_slug)),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        (_, "/v1/health")
        | (_, "/v1/agent/create")
        | (_, "/v1/agent/remove")
        | (_, "/v1/systemd/restart")
        | (_, "/v1/systemd/reload")
        | (_, "/v1/hmac/rotate") => {
            ServiceReply::json_value(405, serde_json::json!({ "error": "method not allowed" }))
        }
        _ => ServiceReply::json_value(404, serde_json::json!({ "error": "not found" })),
    }
}

fn create_agent(request: &CreateAgentRequest) -> ServiceReply {
    if let Err(err) = validate_slug(&request.slug) {
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }
    if request.persona_ref.trim().is_empty() {
        return ServiceReply::json_value(
            400,
            serde_json::json!({ "error": "persona_ref is required" }),
        );
    }

    match run_command(
        "useradd",
        &[
            "--system",
            "--create-home",
            "--shell",
            "/usr/sbin/nologin",
            &request.slug,
        ],
    ) {
        Ok(()) => ServiceReply::json_value(
            200,
            serde_json::json!(OperationReply {
                ok: true,
                operation: "agent.create",
                message: format!("created system user {}", request.slug),
                pending: vec!["systemd unit write", "sudoers setup"],
            }),
        ),
        Err(err) => ServiceReply::json_value(500, serde_json::json!({ "error": err.to_string() })),
    }
}

fn remove_agent(request: &RemoveAgentRequest) -> ServiceReply {
    if let Err(err) = validate_slug(&request.slug) {
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }

    match run_command("userdel", &["--remove", &request.slug]) {
        Ok(()) => ServiceReply::json_value(
            200,
            serde_json::json!(OperationReply {
                ok: true,
                operation: "agent.remove",
                message: format!("removed system user {}", request.slug),
                pending: vec!["systemd disable"],
            }),
        ),
        Err(err) => ServiceReply::json_value(500, serde_json::json!({ "error": err.to_string() })),
    }
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
    if slug.is_empty() || slug.len() > 32 {
        bail!("slug must be 1-32 characters");
    }
    if !slug
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("slug may only contain lowercase ASCII letters, digits, and hyphens");
    }
    Ok(())
}

fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{program} failed with {}: {}", output.status, stderr.trim());
    }
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
    use tokio::net::UnixStream;

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
        };
        let request =
            parse_http_request("GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let reply = handle_request(&state, &request).await;
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
        assert!(validate_slug("AgentKhalid").is_err());
        assert!(validate_slug("agent_khalid").is_err());
    }
}
