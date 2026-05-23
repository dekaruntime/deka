use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[derive(Clone, Debug)]
pub struct GildAgentClient {
    socket_path: PathBuf,
}

#[derive(Debug)]
pub struct AgentClientError {
    status: Option<u16>,
    message: String,
}

impl AgentClientError {
    pub fn status(&self) -> Option<u16> {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AgentClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(f, "gild-agent returned {status}: {}", self.message),
            None => write!(f, "gild-agent request failed: {}", self.message),
        }
    }
}

impl Error for AgentClientError {}

pub type AgentClientResult<T> = std::result::Result<T, AgentClientError>;

#[derive(Debug, Deserialize)]
pub struct UseraddResponse {
    pub ok: bool,
    pub operation: String,
    pub message: String,
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub pending: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserdelResponse {
    pub ok: bool,
    pub operation: String,
    pub message: String,
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub pending: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SystemctlResponse {
    pub ok: bool,
    pub op: String,
    pub action: String,
    pub unit: String,
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub audit_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HmacRotateResponse {
    pub ok: bool,
    pub new_key_fingerprint: String,
}

#[derive(Debug, Deserialize)]
pub struct WriteUnitResponse {
    pub ok: bool,
    pub path: String,
    pub sha256: String,
}

impl GildAgentClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub async fn useradd(&self, slug: &str) -> AgentClientResult<UseraddResponse> {
        #[derive(Serialize)]
        struct Request<'a> {
            slug: &'a str,
            persona_ref: String,
        }

        self.post_json(
            "/v1/agent/create",
            &Request {
                slug,
                persona_ref: format!("personas/{slug}"),
            },
        )
        .await
    }

    pub async fn userdel(&self, slug: &str) -> AgentClientResult<UserdelResponse> {
        #[derive(Serialize)]
        struct Request<'a> {
            slug: &'a str,
        }

        self.post_json("/v1/agent/remove", &Request { slug }).await
    }

    pub async fn systemctl(
        &self,
        action: &str,
        unit: &str,
    ) -> AgentClientResult<SystemctlResponse> {
        #[derive(Serialize)]
        struct Request<'a> {
            op: &'static str,
            action: &'a str,
            unit: &'a str,
        }

        self.post_json(
            "/v1/systemctl",
            &Request {
                op: "systemctl",
                action,
                unit,
            },
        )
        .await
    }

    pub async fn hmac_rotate(&self, slug: &str) -> AgentClientResult<HmacRotateResponse> {
        #[derive(Serialize)]
        struct Request<'a> {
            op: &'static str,
            slug: &'a str,
        }

        self.post_json(
            "/v1/hmac/rotate",
            &Request {
                op: "hmac_rotate",
                slug,
            },
        )
        .await
    }

    pub async fn write_unit(
        &self,
        unit: &str,
        contents: &str,
    ) -> AgentClientResult<WriteUnitResponse> {
        #[derive(Serialize)]
        struct Request<'a> {
            op: &'static str,
            unit: &'a str,
            contents: &'a str,
        }

        self.post_json(
            "/v1/write_unit",
            &Request {
                op: "write_unit",
                unit,
                contents,
            },
        )
        .await
    }

    async fn post_json<T, R>(&self, path: &str, body: &T) -> AgentClientResult<R>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        let body = serde_json::to_vec(body).map_err(client_error)?;
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(client_error)?;
        let request = format!(
            "POST {path} HTTP/1.1\r\nhost: gild-agent\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(client_error)?;
        stream.write_all(&body).await.map_err(client_error)?;
        stream.shutdown().await.map_err(client_error)?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .map_err(client_error)?;
        parse_response(&response)
    }
}

fn parse_response<T>(raw: &[u8]) -> AgentClientResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| AgentClientError {
            status: None,
            message: "malformed HTTP response from gild-agent".to_string(),
        })?;
    let head = std::str::from_utf8(&raw[..header_end]).map_err(client_error)?;
    let status = parse_status(head)?;
    let body = &raw[header_end + 4..];

    if (200..300).contains(&status) {
        return serde_json::from_slice(body).map_err(client_error);
    }

    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| value.get("message").and_then(Value::as_str))
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string());
    Err(AgentClientError {
        status: Some(status),
        message,
    })
}

fn parse_status(head: &str) -> AgentClientResult<u16> {
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| AgentClientError {
            status: None,
            message: "missing HTTP status from gild-agent".to_string(),
        })?;
    status.parse::<u16>().map_err(client_error)
}

fn client_error(err: impl fmt::Display) -> AgentClientError {
    AgentClientError {
        status: None,
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::net::UnixListener;

    fn temp_socket(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "gild-agent-client-{name}-{}-{}.sock",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn serve_once(
        socket: PathBuf,
        response_status: u16,
        response_body: &'static str,
    ) -> tokio::task::JoinHandle<String> {
        let _ = fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut buf = [0_u8; 1024];
                let read = stream.read(&mut buf).await.unwrap();
                assert_ne!(read, 0, "client closed before request headers");
                request.extend_from_slice(&buf[..read]);
                if let Some(pos) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break pos;
                }
            };

            let head = std::str::from_utf8(&request[..header_end]).unwrap();
            let content_length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let body_start = header_end + 4;
            while request.len() - body_start < content_length {
                let mut buf = [0_u8; 1024];
                let read = stream.read(&mut buf).await.unwrap();
                assert_ne!(read, 0, "client closed before request body");
                request.extend_from_slice(&buf[..read]);
            }

            let reason = if response_status == 200 {
                "OK"
            } else {
                "Conflict"
            };
            let response = format!(
                "HTTP/1.1 {response_status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            let _ = fs::remove_file(&socket);
            String::from_utf8(request).unwrap()
        })
    }

    #[tokio::test]
    async fn useradd_serializes_request_and_parses_response() {
        let socket = temp_socket("useradd");
        let server = serve_once(
            socket.clone(),
            200,
            r#"{"ok":true,"operation":"agent.create","message":"created Linux user agent-zed","exit":0,"stdout":"","stderr":"","error":null,"pending":["systemd unit write"]}"#,
        );
        let client = GildAgentClient::new(&socket);

        let response = client.useradd("agent-zed").await.unwrap();
        let request = server.await.unwrap();

        assert!(response.ok);
        assert_eq!(response.operation, "agent.create");
        assert!(request.starts_with("POST /v1/agent/create HTTP/1.1"));
        assert!(request.contains(r#""slug":"agent-zed""#));
        assert!(request.contains(r#""persona_ref":"personas/agent-zed""#));
    }

    #[tokio::test]
    async fn userdel_serializes_request_and_parses_response() {
        let socket = temp_socket("userdel");
        let server = serve_once(
            socket.clone(),
            200,
            r#"{"ok":true,"operation":"agent.remove","message":"removed Linux user agent-zed","exit":0,"stdout":"","stderr":"","error":null,"pending":["systemd disable"]}"#,
        );
        let client = GildAgentClient::new(&socket);

        let response = client.userdel("agent-zed").await.unwrap();
        let request = server.await.unwrap();

        assert!(response.ok);
        assert_eq!(response.operation, "agent.remove");
        assert!(request.starts_with("POST /v1/agent/remove HTTP/1.1"));
        assert!(request.contains(r#""slug":"agent-zed""#));
    }

    #[tokio::test]
    async fn systemctl_serializes_action_and_unit() {
        let socket = temp_socket("systemctl");
        let server = serve_once(
            socket.clone(),
            200,
            r#"{"ok":true,"op":"systemctl","action":"enable","unit":"gg.tana.gild-dispatcher@agent-zed.service","exit":0,"stdout":"","stderr":"","error":null,"audit_error":null}"#,
        );
        let client = GildAgentClient::new(&socket);

        let response = client
            .systemctl("enable", "gg.tana.gild-dispatcher@agent-zed.service")
            .await
            .unwrap();
        let request = server.await.unwrap();

        assert!(response.ok);
        assert_eq!(response.action, "enable");
        assert!(request.starts_with("POST /v1/systemctl HTTP/1.1"));
        assert!(request.contains(r#""op":"systemctl""#));
        assert!(request.contains(r#""action":"enable""#));
    }

    #[tokio::test]
    async fn write_unit_conflict_preserves_status_and_message() {
        let socket = temp_socket("conflict");
        let server = serve_once(
            socket.clone(),
            409,
            r#"{"error":"unit already exists with different sha256"}"#,
        );
        let client = GildAgentClient::new(&socket);

        let err = client
            .write_unit("gg.tana.gild-dispatcher@agent-zed.service", "[Unit]\n")
            .await
            .unwrap_err();
        let request = server.await.unwrap();

        assert_eq!(err.status(), Some(409));
        assert_eq!(err.message(), "unit already exists with different sha256");
        assert!(request.starts_with("POST /v1/write_unit HTTP/1.1"));
    }

    #[tokio::test]
    async fn hmac_rotate_parses_fingerprint() {
        let socket = temp_socket("hmac");
        let server = serve_once(
            socket.clone(),
            200,
            r#"{"ok":true,"new_key_fingerprint":"sha256:abc"}"#,
        );
        let client = GildAgentClient::new(&socket);

        let response = client.hmac_rotate("agent-zed").await.unwrap();
        let request = server.await.unwrap();

        assert!(response.ok);
        assert_eq!(response.new_key_fingerprint, "sha256:abc");
        assert!(request.starts_with("POST /v1/hmac/rotate HTTP/1.1"));
    }
}
