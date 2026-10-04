//! Buffered HTTP transport shared by the async fetch global and synchronous module.
use crate::Result;
use reqwest::{Client, Method, Url, header::HeaderMap};
use std::time::Duration;

pub(crate) const FETCH_BODY_LIMIT: usize = 16 * 1024 * 1024;
pub(crate) const MODULE_BODY_LIMIT: usize = 256 * 4096;

pub(crate) struct BodyPolicy {
    pub limit: usize,
    pub reject_chunked: bool,
}
fn limit_error(limit: usize) -> String {
    if limit == FETCH_BODY_LIMIT {
        "response body exceeds 16 MiB buffered limit".into()
    } else {
        format!("response body exceeds {limit} byte buffered limit")
    }
}
pub(crate) struct Response {
    pub status: u16,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
}

pub(crate) fn client(redirects: bool) -> Result<Client> {
    Client::builder()
        // Runtime configuration is explicit: never consult HTTP_PROXY et al.
        .no_proxy()
        .redirect(if redirects {
            reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    return attempt.error("too many redirects");
                }
                if let Err(error) = validate_url(attempt.url()) {
                    return attempt.error(error);
                }
                attempt.follow()
            })
        } else {
            reqwest::redirect::Policy::none()
        })
        .build()
        .map_err(|error| format!("build HTTP client: {error}"))
}

pub(crate) fn validate_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("fetch supports only http and https URLs".into());
    }
    if !url.username().is_empty() || url.password().is_some_and(|p| !p.is_empty()) {
        return Err("request URL must not contain credentials".into());
    }
    Ok(())
}

pub(crate) async fn request(
    client: Client,
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
    policy: BodyPolicy,
    timeout: Option<Duration>,
) -> Result<Response> {
    validate_url(&url)?;
    let limit = policy.limit;
    if body.len() > limit {
        return Err(format!("request body exceeds {limit} byte buffered limit"));
    }
    let mut builder = client.request(method, url).headers(headers).body(body);
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    let mut response = builder.send().await.map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let url = response.url().to_string();
    let headers = response.headers().clone();
    if policy.reject_chunked
        && headers
            .get(reqwest::header::TRANSFER_ENCODING)
            .is_some_and(|v| {
                v.as_bytes()
                    .windows(7)
                    .any(|w| w.eq_ignore_ascii_case(b"chunked"))
            })
    {
        return Err("chunked encoding not supported".into());
    }
    let body = if matches!(status, 204 | 205 | 304) {
        None
    } else {
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err(limit_error(limit));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
            if chunk.len() > limit - bytes.len() {
                return Err(limit_error(limit));
            }
            bytes
                .try_reserve(chunk.len())
                .map_err(|_| "response body allocation failed")?;
            bytes.extend_from_slice(&chunk);
        }
        Some(bytes)
    };
    Ok(Response {
        status,
        url,
        headers,
        body,
    })
}

pub(crate) fn synchronous(
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
) -> Result<Response> {
    // A separate worker owns its runtime. This remains safe when the calling VM
    // is already driven by Tokio; the canonical module API is synchronous.
    std::thread::Builder::new()
        .name("deka-http".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("HTTP runtime: {e}"))?;
            runtime.block_on(async {
                request(
                    client(false)?,
                    method,
                    url,
                    headers,
                    body,
                    BodyPolicy {
                        limit: MODULE_BODY_LIMIT,
                        reject_chunked: true,
                    },
                    Some(Duration::from_secs(10)),
                )
                .await
            })
        })
        .map_err(|e| format!("HTTP worker: {e}"))?
        .join()
        .map_err(|_| "HTTP worker failed".to_owned())?
}
