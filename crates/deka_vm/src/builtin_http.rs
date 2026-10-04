//! Canonical synchronous HTTP client module, using the fetch transport.
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};
use reqwest::{
    Method, Url,
    header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST, HeaderMap, HeaderValue},
};

fn record(fields: impl IntoIterator<Item = (&'static str, HostType)>) -> HostType {
    HostType::Record(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn url_type() -> HostType {
    record([
        ("scheme", HostType::String),
        ("host", HostType::String),
        ("port", HostType::Number),
        ("path", HostType::String),
    ])
}
fn headers_type() -> HostType {
    record([("content_type", HostType::String)])
}
fn response_type() -> HostType {
    record([
        ("complete", HostType::Bool),
        ("error", HostType::Option(Box::new(HostType::String))),
        ("status", HostType::Number),
        ("body", HostType::Bytes),
    ])
}
fn text(v: &HostValue) -> &str {
    let HostValue::String(v) = v else {
        unreachable!("checked string")
    };
    v
}
fn bytes(v: &HostValue) -> &[u8] {
    let HostValue::Bytes(v) = v else {
        unreachable!("checked bytes")
    };
    v
}
fn content_type(v: &HostValue) -> &str {
    let HostValue::Record(v) = v else {
        unreachable!("checked Headers")
    };
    text(&v["content_type"])
}

/// Preserve the shipped parser's default scheme, bracketed IPv6 and path rules.
pub fn parse_url(raw: &str) -> Result<HostValue> {
    let (scheme, rest) = raw.split_once("://").unwrap_or(("http", raw));
    let (authority, path) = rest
        .find('/')
        .map_or((rest, "/"), |n| (&rest[..n], &rest[n..]));
    let default = if scheme == "https" { 443. } else { 80. };
    let (host, port) = if let Some(ipv6) = authority.strip_prefix('[') {
        let end = ipv6.find(']').ok_or("invalid url")?;
        let after = &ipv6[end + 1..];
        (
            &ipv6[..end],
            if let Some(p) = after.strip_prefix(':') {
                number(p)?
            } else {
                default
            },
        )
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        (h, number(p)?)
    } else {
        (authority, default)
    };
    if host.is_empty() {
        return Err("invalid url".into());
    }
    Ok(HostValue::Record(
        [
            ("scheme".into(), HostValue::String(scheme.into())),
            ("host".into(), HostValue::String(host.into())),
            ("port".into(), HostValue::Number(port)),
            ("path".into(), HostValue::String(path.into())),
        ]
        .into(),
    ))
}
fn number(input: &str) -> Result<f64> {
    let input = input.trim();
    let value = if input.is_empty() {
        0.
    } else {
        if let Some((digits, radix)) = input
            .strip_prefix("0x")
            .or_else(|| input.strip_prefix("0X"))
            .map(|v| (v, 16))
            .or_else(|| {
                input
                    .strip_prefix("0b")
                    .or_else(|| input.strip_prefix("0B"))
                    .map(|v| (v, 2))
            })
            .or_else(|| {
                input
                    .strip_prefix("0o")
                    .or_else(|| input.strip_prefix("0O"))
                    .map(|v| (v, 8))
            })
        {
            if digits.is_empty() {
                return Err("invalid url".into());
            }
            digits.chars().try_fold(0., |value, c| {
                c.to_digit(radix)
                    .map(|d| value * f64::from(radix) + f64::from(d))
                    .ok_or("invalid url")
            })?
        } else {
            input.parse::<f64>().map_err(|_| "invalid url")?
        }
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err("invalid url".into())
    }
}
pub fn format_request(
    method: &str,
    path: &str,
    host: &str,
    content_type: &str,
    body: &[u8],
) -> String {
    format!(
        "{} {} HTTP/1.1\r\nContent-Type: {}\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        method.to_uppercase(),
        if path.is_empty() { "/" } else { path },
        content_type,
        host,
        body.len()
    )
}
fn request(method: &str, input: &str, content_type: &str, body: &[u8]) -> Result<HostValue> {
    let HostValue::Record(parsed) = parse_url(input).map_err(|_| "invalid url")? else {
        unreachable!()
    };
    let scheme = text(&parsed["scheme"]);
    let host = text(&parsed["host"]);
    let path = text(&parsed["path"]);
    let HostValue::Number(port) = parsed["port"] else {
        unreachable!()
    };
    if port.fract() != 0. || !(0. ..=65535.).contains(&port) {
        return Err("invalid url".into());
    }
    let authority = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.into()
    };
    let url = Url::parse(&format!("{scheme}://{authority}:{}{path}", port as u16))
        .map_err(|_| "invalid url")?;
    let method = Method::from_bytes(method.to_uppercase().as_bytes()).map_err(|e| e.to_string())?;
    let mut headers = HeaderMap::new();
    for (name, value) in [
        (CONTENT_TYPE, content_type.into()),
        (HOST, host.into()),
        (CONTENT_LENGTH, body.len().to_string()),
        (CONNECTION, "close".into()),
    ] {
        headers.insert(
            name,
            HeaderValue::from_str(&value).map_err(|e| e.to_string())?,
        );
    }
    if body.len() > crate::http_transport::MODULE_BODY_LIMIT {
        return Err("request body exceeds 1048576 byte buffered limit".into());
    }
    let response = crate::http_transport::synchronous(method, url, headers, body.to_vec())?;
    Ok(HostValue::Record(
        [
            ("complete".into(), HostValue::Bool(true)),
            ("error".into(), HostValue::Option(None)),
            ("status".into(), HostValue::Number(response.status.into())),
            (
                "body".into(),
                HostValue::Bytes(response.body.unwrap_or_default()),
            ),
        ]
        .into(),
    ))
}
#[derive(Clone, Copy)]
enum Function {
    Parse,
    Format,
    Request,
    Get,
    Post,
}
impl Function {
    const ALL: [Self; 5] = [
        Self::Parse,
        Self::Format,
        Self::Request,
        Self::Get,
        Self::Post,
    ];
    fn declaration(self) -> (&'static str, Vec<HostType>, HostType) {
        use HostType::{Bytes, String};
        match self {
            Self::Parse => ("parse_url", vec![String], url_type()),
            Self::Format => (
                "format_request",
                vec![String, String, String, headers_type(), Bytes],
                String,
            ),
            Self::Request => (
                "request",
                vec![String, String, headers_type(), Bytes],
                response_type(),
            ),
            Self::Get => ("get", vec![String], response_type()),
            Self::Post => ("post", vec![String, String, Bytes], response_type()),
        }
    }
    fn invoke(self, a: &[HostValue]) -> Result<HostValue> {
        match self {
            Self::Parse => parse_url(text(&a[0])),
            Self::Format => Ok(HostValue::String(format_request(
                text(&a[0]),
                text(&a[1]),
                text(&a[2]),
                content_type(&a[3]),
                bytes(&a[4]),
            ))),
            Self::Request => request(text(&a[0]), text(&a[1]), content_type(&a[2]), bytes(&a[3])),
            Self::Get => request("GET", text(&a[0]), "", &[]),
            Self::Post => request("POST", text(&a[0]), text(&a[1]), bytes(&a[2])),
        }
    }
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    for function in Function::ALL {
        let (name, args, result) = function.declaration();
        let op = HostOp::new(&format!("http_{name}"), args, result, false, move |args| {
            HostReply::Ready(function.invoke(&args))
        });
        hosts.register(match function {
            Function::Parse => op.with_exception_channel(),
            Function::Format => op,
            _ => op.with_result_channel(),
        })?;
    }
    Ok(())
}
