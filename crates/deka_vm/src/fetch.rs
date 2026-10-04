//! Native fetch uses the VM's Tokio future; no blocking bridge/runtime registry.
use crate::http_headers::HeaderList;
use crate::http_request::RequestObject;
use crate::http_response::ResponseObject;
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};
use reqwest::{Client, Url};

// Preserve the old Rust transport's buffered limits, without buffering beyond
// the cap before checking it. Streaming is a separate host surface.
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_REDIRECTS: usize = 10;

fn transport_url(input: &str) -> Result<Url> {
    let request = RequestObject::new(input)?;
    let mut url = Url::parse(request.url()).map_err(|error| error.to_string())?;
    validate_url(&url)?;
    url.set_fragment(None);
    Ok(url)
}
fn validate_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("fetch supports only http and https URLs".into());
    }
    if !url.username().is_empty() || url.password().is_some_and(|password| !password.is_empty()) {
        return Err("request URL must not contain credentials".into());
    }
    Ok(())
}
fn client() -> Result<Client> {
    Client::builder()
        // Product configuration is explicit. Do not consult HTTP_PROXY et al.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            if let Err(error) = validate_url(attempt.url()) {
                return attempt.error(error);
            }
            attempt.follow()
        }))
        .build()
        .map_err(|error| format!("build HTTP client: {error}"))
}
async fn get(client: Client, url: Url) -> Result<HostValue> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let url = response.url().to_string();
    let mut headers = HeaderList::default();
    for (name, value) in response.headers() {
        // Header byte strings use the same isomorphic Latin-1 mapping as
        // HeaderList::wire_entries; do not round-trip through lossy UTF-8.
        let value = value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect::<String>();
        headers.append(name.as_str(), &value)?;
    }
    let body = if matches!(status, 204 | 205 | 304) {
        None
    } else {
        if response
            .content_length()
            .is_some_and(|size| size > MAX_BODY_BYTES as u64)
        {
            return Err("response body exceeds 16 MiB buffered limit".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
            if chunk.len() > MAX_BODY_BYTES - body.len() {
                return Err("response body exceeds 16 MiB buffered limit".into());
            }
            body.try_reserve(chunk.len())
                .map_err(|_| "response body allocation failed")?;
            body.extend_from_slice(&chunk);
        }
        Some(body)
    };
    // reqwest does not expose the original HTTP reason phrase. Leave it empty
    // rather than substituting a canonical phrase the server did not send.
    ResponseObject::from_parts(status, String::new(), url, headers, body)
        .map(ResponseObject::into_host_value)
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    let client = client()?;
    hosts.register(
        HostOp::new(
            "fetch",
            vec![
                HostType::String,
                HostType::Record(
                    [(
                        "signal".into(),
                        HostType::Option(Box::new(HostType::Handle("AbortSignal".into()))),
                    )]
                    .into(),
                ),
            ],
            HostType::Handle("Response".into()),
            true,
            move |args| {
                let HostValue::String(input) = &args[0] else {
                    unreachable!("checked fetch URL")
                };
                let HostValue::Record(init) = &args[1] else {
                    unreachable!("checked fetch options")
                };
                let signal = match &init["signal"] {
                    HostValue::Option(None) => None,
                    HostValue::Option(Some(value)) => {
                        let HostValue::Handle(handle) = &**value else {
                            unreachable!("checked fetch signal")
                        };
                        let Some(signal) = handle.downcast_ref::<crate::abort::AbortSignalObject>()
                        else {
                            return HostReply::Ready(Err("invalid AbortSignal resource".into()));
                        };
                        Some(signal.clone())
                    }
                    _ => unreachable!("checked fetch signal option"),
                };
                if let Some(reason) = signal.as_ref().and_then(|signal| signal.reason()) {
                    return HostReply::Ready(Err(reason));
                }
                match transport_url(input) {
                    Ok(url) => {
                        let request = get(client.clone(), url);
                        HostReply::Pending(Box::pin(async move {
                            if let Some(signal) = signal {
                                tokio::select! {
                                    biased;
                                    reason = signal.cancelled() => Err(reason),
                                    response = request => response,
                                }
                            } else {
                                request.await
                            }
                        }))
                    }
                    Err(error) => HostReply::Ready(Err(error)),
                }
            },
        )
        .with_defaults(vec![HostValue::Record(Default::default())])
        .with_global_binding()
        .with_result_channel(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_urls_share_request_normalization_and_drop_fragments() {
        assert_eq!(
            transport_url("https://EXAMPLE.test:443/a/../b#local")
                .unwrap()
                .as_str(),
            "https://example.test/b"
        );
        for input in [
            "/relative",
            "https://user:password@example.test/",
            "file:///tmp/file",
            "data:text/plain,a",
        ] {
            assert!(transport_url(input).is_err(), "{input}");
        }
    }
}
