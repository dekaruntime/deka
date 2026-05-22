use anyhow::{Context, Result, anyhow, bail};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

const DEFAULT_SOCKET_PATH: &str = "/run/tana-vault.sock";
const DEFAULT_CONFIG_DIR: &str = "/etc/tana-vault-agent";
const TOKEN_TTL_SECONDS: u64 = 300;

#[derive(Debug, Clone)]
struct AppState {
    config: AgentConfig,
    workloads: WorkloadsConfig,
    jwt_key: Arc<Vec<u8>>,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize)]
struct AgentConfig {
    #[serde(default = "default_socket_path")]
    socket_path: PathBuf,
    vault_url: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from(DEFAULT_SOCKET_PATH),
            vault_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct WorkloadsConfig {
    workloads: Vec<WorkloadRule>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkloadRule {
    identity: String,
    namespace: String,
    exe: PathBuf,
    #[serde(default)]
    cgroup_contains: Vec<String>,
    #[serde(default)]
    systemd_unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessIdentity {
    pid: u32,
    exe: PathBuf,
    cgroup: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWorkload {
    identity: String,
    namespace: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct VaultClaims {
    sub: String,
    scope: String,
    iat: u64,
    exp: u64,
}

#[derive(Debug, Deserialize)]
struct VaultSecretResponse {
    value: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config_dir = std::env::var("TANA_VAULT_AGENT_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CONFIG_DIR));
    let state = Arc::new(load_state(&config_dir).await?);
    serve(state).await
}

async fn load_state(config_dir: &Path) -> Result<AppState> {
    let config = load_agent_config(&config_dir.join("agent.toml"))?;
    let workloads = load_workloads(&config_dir.join("workloads.toml"))?;
    let jwt_key = load_hs256_key(&config_dir.join("dev-key"))?;

    Ok(AppState {
        config,
        workloads,
        jwt_key: Arc::new(jwt_key),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .context("build HTTP client")?,
    })
}

async fn serve(state: Arc<AppState>) -> Result<()> {
    prepare_socket_path(&state.config.socket_path)?;
    let listener = UnixListener::bind(&state.config.socket_path)
        .with_context(|| format!("bind {}", state.config.socket_path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(&state.config.socket_path, fs::Permissions::from_mode(0o666))
        .with_context(|| format!("chmod 0666 {}", state.config.socket_path.display()))?;

    eprintln!(
        "tana-vault-agent listening on {}",
        state.config.socket_path.display()
    );

    loop {
        let (stream, _) = listener.accept().await.context("accept unix connection")?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, state).await {
                eprintln!("tana-vault-agent connection error: {err:#}");
            }
        });
    }
}

async fn handle_connection(mut stream: UnixStream, state: Arc<AppState>) -> Result<()> {
    let peer = peer_process_identity(&stream)?;
    let workload = resolve_workload(&state.workloads, &peer).ok_or_else(|| {
        anyhow!(
            "no workload mapping for pid={} exe={} cgroup={:?}",
            peer.pid,
            peer.exe.display(),
            peer.cgroup
        )
    })?;

    let request = read_http_request(&mut stream).await?;
    let response = match handle_request(&state, &workload, &request).await {
        Ok(body) => http_response(200, &body),
        Err(err) => {
            eprintln!("tana-vault-agent request error: {err:#}");
            http_response(
                500,
                &serde_json::json!({ "error": "tana-vault-agent request failed" }).to_string(),
            )
        }
    };
    stream
        .write_all(response.as_bytes())
        .await
        .context("write response")?;
    Ok(())
}

async fn handle_request(
    state: &AppState,
    workload: &ResolvedWorkload,
    request: &HttpRequest,
) -> Result<String> {
    if request.method != "GET" {
        return Ok(serde_json::json!({ "error": "method not allowed" }).to_string());
    }

    let key = request
        .path
        .strip_prefix("/v1/secret/")
        .filter(|key| !key.is_empty())
        .ok_or_else(|| anyhow!("unknown route {}", request.path))?;

    let token = mint_jwt(&state.jwt_key, workload, now_epoch_seconds()?)?;
    eprintln!(
        "minted vault token for sub={} scope=vault:read:{}/* key={}",
        workload.identity, workload.namespace, key
    );

    match fetch_from_vault(state, key, &token).await {
        Ok(value) => Ok(serde_json::json!({ "value": value }).to_string()),
        Err(err) => {
            eprintln!("vault upstream unavailable, returning stub: {err:#}");
            Ok(serde_json::json!({ "value": format!("stub-{key}") }).to_string())
        }
    }
}

async fn fetch_from_vault(state: &AppState, key: &str, token: &str) -> Result<String> {
    let base = state
        .config
        .vault_url
        .as_deref()
        .ok_or_else(|| anyhow!("vault_url is not configured"))?
        .trim_end_matches('/');
    let url = format!("{base}/v1/secret/{key}");
    let response = state
        .client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .context("send vault request")?
        .error_for_status()
        .context("vault returned error status")?;
    let body = response
        .json::<VaultSecretResponse>()
        .await
        .context("decode vault secret response")?;
    Ok(body.value)
}

#[derive(Debug, PartialEq, Eq)]
struct HttpRequest {
    method: String,
    path: String,
}

async fn read_http_request(stream: &mut UnixStream) -> Result<HttpRequest> {
    let mut buf = vec![0; 8192];
    let mut used = 0;
    loop {
        if used == buf.len() {
            bail!("request headers too large");
        }
        let n = stream
            .read(&mut buf[used..])
            .await
            .context("read request")?;
        if n == 0 {
            bail!("client closed before request headers");
        }
        used += n;
        if buf[..used].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    let raw = std::str::from_utf8(&buf[..used]).context("request is not UTF-8")?;
    parse_http_request(raw)
}

fn parse_http_request(raw: &str) -> Result<HttpRequest> {
    let request_line = raw
        .lines()
        .next()
        .ok_or_else(|| anyhow!("empty HTTP request"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP method"))?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP path"))?
        .to_string();
    Ok(HttpRequest { method, path })
}

fn http_response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        500 => "Internal Server Error",
        _ => "OK",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\n\r\n{body}",
        body.len()
    )
}

fn load_agent_config(path: &Path) -> Result<AgentConfig> {
    match fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw).with_context(|| format!("parse {}", path.display())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(AgentConfig::default()),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

fn load_workloads(path: &Path) -> Result<WorkloadsConfig> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn load_hs256_key(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    #[cfg(unix)]
    {
        let mode = metadata.mode() & 0o777;
        if mode != 0o600 {
            bail!("{} must have mode 0600, found {:03o}", path.display(), mode);
        }
    }
    let key = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if key.is_empty() {
        bail!("{} is empty", path.display());
    }
    Ok(key)
}

fn resolve_workload(
    config: &WorkloadsConfig,
    process: &ProcessIdentity,
) -> Option<ResolvedWorkload> {
    config
        .workloads
        .iter()
        .find(|rule| workload_matches(rule, process))
        .map(|rule| ResolvedWorkload {
            identity: rule.identity.clone(),
            namespace: rule.namespace.clone(),
        })
}

fn workload_matches(rule: &WorkloadRule, process: &ProcessIdentity) -> bool {
    if rule.exe != process.exe {
        return false;
    }

    if let Some(unit) = &rule.systemd_unit
        && !process.cgroup.contains(unit)
    {
        return false;
    }

    rule.cgroup_contains
        .iter()
        .all(|needle| process.cgroup.contains(needle))
}

fn mint_jwt(key: &[u8], workload: &ResolvedWorkload, now: u64) -> Result<String> {
    let claims = VaultClaims {
        sub: workload.identity.clone(),
        scope: format!("vault:read:{}/*", workload.namespace),
        iat: now,
        exp: now + TOKEN_TTL_SECONDS,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(key),
    )
    .context("mint HS256 JWT")
}

fn now_epoch_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before UNIX epoch")?
        .as_secs())
}

fn peer_process_identity(stream: &UnixStream) -> Result<ProcessIdentity> {
    let pid = peer_pid(stream)?;
    let exe = fs::read_link(format!("/proc/{pid}/exe")).context("read peer /proc/PID/exe")?;
    let cgroup =
        fs::read_to_string(format!("/proc/{pid}/cgroup")).context("read peer /proc/PID/cgroup")?;
    Ok(ProcessIdentity { pid, exe, cgroup })
}

fn peer_pid(stream: &UnixStream) -> Result<u32> {
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
    u32::try_from(cred.pid).context("peer pid is negative")
}

fn prepare_socket_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(unix)]
            if metadata.file_type().is_socket() {
                fs::remove_file(path)
                    .with_context(|| format!("remove stale socket {}", path.display()))?;
                return Ok(());
            }
            bail!("{} exists and is not a socket", path.display());
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("stat {}", path.display())),
    }
}

fn default_socket_path() -> PathBuf {
    PathBuf::from(DEFAULT_SOCKET_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode};

    #[test]
    fn resolves_workload_by_exe_and_cgroup() {
        let config = WorkloadsConfig {
            workloads: vec![
                WorkloadRule {
                    identity: "prod/deka.gg".to_string(),
                    namespace: "prod/deka.gg".to_string(),
                    exe: PathBuf::from("/srv/deka/bin/deka"),
                    cgroup_contains: vec!["system.slice".to_string()],
                    systemd_unit: Some("gg.tana.deka-platform.service".to_string()),
                },
                WorkloadRule {
                    identity: "prod/linkhash".to_string(),
                    namespace: "prod/linkhash".to_string(),
                    exe: PathBuf::from("/srv/linkhash/bin/server"),
                    cgroup_contains: Vec::new(),
                    systemd_unit: None,
                },
            ],
        };
        let process = ProcessIdentity {
            pid: 123,
            exe: PathBuf::from("/srv/deka/bin/deka"),
            cgroup: "0::/system.slice/gg.tana.deka-platform.service".to_string(),
        };

        let resolved = resolve_workload(&config, &process).expect("workload should resolve");

        assert_eq!(
            resolved,
            ResolvedWorkload {
                identity: "prod/deka.gg".to_string(),
                namespace: "prod/deka.gg".to_string(),
            }
        );
    }

    #[test]
    fn rejects_workload_when_unit_does_not_match() {
        let config = WorkloadsConfig {
            workloads: vec![WorkloadRule {
                identity: "prod/deka.gg".to_string(),
                namespace: "prod/deka.gg".to_string(),
                exe: PathBuf::from("/srv/deka/bin/deka"),
                cgroup_contains: Vec::new(),
                systemd_unit: Some("gg.tana.deka-platform.service".to_string()),
            }],
        };
        let process = ProcessIdentity {
            pid: 123,
            exe: PathBuf::from("/srv/deka/bin/deka"),
            cgroup: "0::/system.slice/other.service".to_string(),
        };

        assert!(resolve_workload(&config, &process).is_none());
    }

    #[test]
    fn mints_hs256_jwt_with_vault_scope() {
        let workload = ResolvedWorkload {
            identity: "prod/deka.gg".to_string(),
            namespace: "prod/deka.gg".to_string(),
        };

        let token = mint_jwt(b"dev-secret", &workload, 1_779_420_000).expect("mint jwt");
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false;
        let data = decode::<VaultClaims>(
            &token,
            &DecodingKey::from_secret(b"dev-secret"),
            &validation,
        )
        .expect("decode jwt");

        assert_eq!(data.claims.sub, "prod/deka.gg");
        assert_eq!(data.claims.scope, "vault:read:prod/deka.gg/*");
        assert_eq!(data.claims.iat, 1_779_420_000);
        assert_eq!(data.claims.exp, 1_779_420_000 + TOKEN_TTL_SECONDS);
    }

    #[test]
    fn requires_dev_key_mode_0600() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key_path = dir.path().join("dev-key");
        fs::write(&key_path, b"secret").expect("write key");
        #[cfg(unix)]
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).expect("chmod key");

        #[cfg(unix)]
        assert!(load_hs256_key(&key_path).is_err());

        #[cfg(unix)]
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).expect("chmod key");
        assert_eq!(load_hs256_key(&key_path).expect("load key"), b"secret");
    }
}
