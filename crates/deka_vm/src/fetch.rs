//! Native fetch uses the VM's Tokio future; no blocking bridge/runtime registry.
use crate::http_headers::HeaderList;
use crate::http_request::RequestObject;
use crate::http_response::ResponseObject;
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};
use reqwest::{Client, Url};

fn transport_url(input: &str) -> Result<Url> {
    let request = RequestObject::new(input)?;
    let mut url = Url::parse(request.url()).map_err(|error| error.to_string())?;
    crate::http_transport::validate_url(&url)?;
    url.set_fragment(None);
    Ok(url)
}
async fn get(client: Client, url: Url) -> Result<HostValue> {
    let response = crate::http_transport::request(
        client,
        reqwest::Method::GET,
        url,
        Default::default(),
        Vec::new(),
        crate::http_transport::BodyPolicy {
            limit: crate::http_transport::FETCH_BODY_LIMIT,
            reject_chunked: false,
        },
        None,
    )
    .await?;
    let status = response.status;
    let url = response.url;
    let mut headers = HeaderList::default();
    for (name, value) in &response.headers {
        // Header byte strings use the same isomorphic Latin-1 mapping as
        // HeaderList::wire_entries; do not round-trip through lossy UTF-8.
        let value = value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect::<String>();
        headers.append(name.as_str(), &value)?;
    }
    let body = response.body;
    // reqwest does not expose the original HTTP reason phrase. Leave it empty
    // rather than substituting a canonical phrase the server did not send.
    ResponseObject::from_parts(status, String::new(), url, headers, body)
        .map(ResponseObject::into_host_value)
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    let client = crate::http_transport::client(true)?;
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
