use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct GildExecuteRequest {
    pub run_id: String,
    pub agent_slug: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub stdin: String,
    pub env: HashMap<String, String>,
    pub mounts: Vec<Mount>,
    pub network_policy: NetworkPolicy,
    pub vsock_callbacks: Vec<VsockCallback>,
    pub timeout_ms: u64,
    pub memory: String,
}

#[derive(Debug, Serialize)]
pub struct Mount {
    pub host: String,
    pub guest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ro: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct NetworkPolicy {
    pub allow_ipsets: Vec<String>,
    pub deny_cidrs: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct VsockCallback {
    pub name: String,
    pub target: String,
    pub auth: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GildExecuteResponse {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub run_id: String,
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum GildError {
    Unreachable(String),
    Http(u16, String),
    Protocol(String),
}

impl std::fmt::Display for GildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GildError::Unreachable(msg) => write!(f, "gild unreachable: {}", msg),
            GildError::Http(code, body) => write!(f, "gild HTTP {}: {}", code, body),
            GildError::Protocol(msg) => write!(f, "gild protocol error: {}", msg),
        }
    }
}

pub async fn gild_execute_unix(
    req: &GildExecuteRequest,
    socket_path: &str,
    bearer_token: &str,
) -> Result<GildExecuteResponse, GildError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let body = serde_json::to_string(req).map_err(|e| {
        GildError::Protocol(format!("failed to serialize request: {}", e))
    })?;

    let request = format!(
        "POST /execute HTTP/1.1\r\n\
         Host: localhost\r\n\
         Authorization: Bearer {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        bearer_token,
        body.len(),
        body
    );

    let stream = UnixStream::connect(socket_path).await.map_err(|e| {
        GildError::Unreachable(format!("connect to {}: {}", socket_path, e))
    })?;

    let (mut reader, mut writer) = stream.into_split();

    writer.write_all(request.as_bytes()).await.map_err(|e| {
        GildError::Unreachable(format!("write to gild socket: {}", e))
    })?;
    writer.shutdown().await.map_err(|e| {
        GildError::Unreachable(format!("shutdown gild socket: {}", e))
    })?;
    drop(writer);

    let mut response_buf = Vec::new();
    reader.read_to_end(&mut response_buf).await.map_err(|e| {
        GildError::Unreachable(format!("read from gild socket: {}", e))
    })?;

    parse_http_response(&response_buf)
}

fn parse_http_response(raw: &[u8]) -> Result<GildExecuteResponse, GildError> {
    let text = std::str::from_utf8(raw)
        .map_err(|e| GildError::Protocol(format!("invalid utf-8 response: {}", e)))?;

    let (status_line, rest) = text.split_once("\r\n").unwrap_or((text, ""));
    let status_parts: Vec<&str> = status_line.split_whitespace().collect();
    let status_code: u16 = status_parts
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    let (headers, body) = if let Some(idx) = rest.find("\r\n\r\n") {
        let (h, b) = rest.split_at(idx);
        (h, &b[4..])
    } else {
        ("", rest)
    };

    let content_length: usize = headers
        .lines()
        .find_map(|line| {
            if line.to_ascii_lowercase().starts_with("content-length:") {
                line.split(':').nth(1).and_then(|v| v.trim().parse().ok())
            } else {
                None
            }
        })
        .unwrap_or(body.len());

    let body = &body[..content_length.min(body.len())];

    if status_code >= 200 && status_code < 300 {
        let response: GildExecuteResponse =
            serde_json::from_str(body).map_err(|e| {
                GildError::Protocol(format!("parse response json: {}", e))
            })?;
        Ok(response)
    } else if status_code == 502 || status_code == 503 || status_code == 504 {
        Err(GildError::Unreachable(format!(
            "gild HTTP {}",
            status_code
        )))
    } else {
        Err(GildError::Http(status_code, body.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_successful_http_response() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 123\r\n\r\n{\"ok\":true,\"stdout\":\"hello\",\"stderr\":\"\",\"exit_code\":0,\"duration_ms\":42,\"run_id\":\"run-1\"}";
        let response = parse_http_response(raw).unwrap();
        assert!(response.ok);
        assert_eq!(response.stdout, "hello");
        assert_eq!(response.stderr, "");
        assert_eq!(response.exit_code, 0);
        assert_eq!(response.duration_ms, 42);
        assert_eq!(response.run_id, "run-1");
    }

    #[test]
    fn parses_failed_http_response() {
        let body = "{\"ok\":false,\"stdout\":\"\",\"stderr\":\"error\",\"exit_code\":1,\"duration_ms\":10,\"run_id\":\"run-1\",\"error\":\"command failed\"}";
        let raw = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let response = parse_http_response(raw.as_bytes()).unwrap();
        assert!(!response.ok);
        assert_eq!(response.exit_code, 1);
        assert_eq!(response.error.as_deref(), Some("command failed"));
    }

    #[test]
    fn handles_502_unreachable() {
        let raw = b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n";
        let err = parse_http_response(raw).unwrap_err();
        assert!(matches!(err, GildError::Unreachable(_)));
    }

    #[test]
    fn handles_503_unreachable() {
        let raw = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
        let err = parse_http_response(raw).unwrap_err();
        assert!(matches!(err, GildError::Unreachable(_)));
    }

    #[test]
    fn handles_generic_http_error() {
        let raw = b"HTTP/1.1 400 Bad Request\r\nContent-Length: 15\r\n\r\n{\"error\":\"bad\"}";
        let err = parse_http_response(raw).unwrap_err();
        assert!(matches!(err, GildError::Http(400, _)));
    }

    #[test]
    fn serializes_request_with_all_fields() {
        let req = GildExecuteRequest {
            run_id: "test-run".to_string(),
            agent_slug: "deka-deploy".to_string(),
            argv: vec!["echo".to_string(), "hello".to_string()],
            cwd: "/workspace".to_string(),
            stdin: "".to_string(),
            env: [("KEY".to_string(), "value".to_string())].into(),
            mounts: vec![Mount {
                host: "/host".to_string(),
                guest: "/guest".to_string(),
                ro: Some(true),
            }],
            network_policy: NetworkPolicy {
                allow_ipsets: vec!["llm-providers".to_string()],
                deny_cidrs: vec!["10.0.0.0/8".to_string()],
            },
            vsock_callbacks: vec![],
            timeout_ms: 60000,
            memory: "2048M".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"run_id\":\"test-run\""));
        assert!(json.contains("\"agent_slug\":\"deka-deploy\""));
        assert!(json.contains("\"argv\":[\"echo\",\"hello\"]"));
        assert!(json.contains("\"cwd\":\"/workspace\""));
        assert!(json.contains("\"KEY\":\"value\""));
        assert!(json.contains("\"host\":\"/host\""));
        assert!(json.contains("\"guest\":\"/guest\""));
        assert!(json.contains("\"ro\":true"));
        assert!(json.contains("\"allow_ipsets\":[\"llm-providers\"]"));
        assert!(json.contains("\"deny_cidrs\":[\"10.0.0.0/8\"]"));
        assert!(json.contains("\"timeout_ms\":60000"));
        assert!(json.contains("\"memory\":\"2048M\""));
    }

    #[test]
    fn serializes_request_without_ro_field_when_none() {
        let req = GildExecuteRequest {
            run_id: "r".to_string(),
            agent_slug: "a".to_string(),
            argv: vec![],
            cwd: "/".to_string(),
            stdin: "".to_string(),
            env: HashMap::new(),
            mounts: vec![Mount {
                host: "/h".to_string(),
                guest: "/g".to_string(),
                ro: None,
            }],
            network_policy: NetworkPolicy {
                allow_ipsets: vec![],
                deny_cidrs: vec![],
            },
            vsock_callbacks: vec![],
            timeout_ms: 1000,
            memory: "512M".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"ro\""));
    }
}
