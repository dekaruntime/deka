use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::config::{MAX_BODY_BYTES, MAX_HEADER_BYTES};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ServiceReply {
    pub(crate) status: u16,
    pub(crate) body: String,
}
pub(crate) fn parse_json<T: for<'de> Deserialize<'de>>(request: &HttpRequest) -> Result<T> {
    if request.body.is_empty() {
        bail!("request body is required");
    }
    serde_json::from_slice(&request.body).context("parse JSON body")
}

pub(crate) async fn read_http_request(stream: &mut UnixStream) -> Result<HttpRequest> {
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
pub(crate) fn parse_http_request(raw: &str) -> Result<HttpRequest> {
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

pub(crate) fn http_response(status: u16, body: &str) -> String {
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

pub(crate) async fn write_reply(stream: &mut UnixStream, reply: ServiceReply) -> Result<()> {
    stream
        .write_all(http_response(reply.status, &reply.body).as_bytes())
        .await
        .context("write response")
}

impl ServiceReply {
    pub(crate) fn json_value(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            body: value.to_string(),
        }
    }
}
