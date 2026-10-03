//! Rust-owned request metadata and buffered transport snapshots.
//! Source construction initially supports the settled default-GET slice.
use crate::{HostHandle, Result, http_headers::HeaderList, url::UrlObject};
use std::cell::RefCell;

/// A transport gets a snapshot of owned data, independent of later header edits.
/// The body stays buffered; streams and source initializer forms are separate.
#[derive(Clone, Debug, PartialEq)]
pub struct BufferedRequest {
    pub url: String,
    pub method: String,
    pub headers: HeaderList,
    pub body: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct RequestObject {
    url: String,
    headers: HostHandle,
    body: Option<Vec<u8>>,
}
impl RequestObject {
    pub fn new(input: &str) -> Result<Self> {
        let url = UrlObject::parse(input, None)?;
        if !url.username().is_empty() || !url.password().is_empty() {
            return Err("request URL must not contain credentials".into());
        }
        let crate::HostValue::Handle(headers) = crate::http_headers::handle(HeaderList::default())
        else {
            unreachable!("shared Headers constructor returns its opaque handle")
        };
        Ok(Self {
            url: url.href(),
            headers,
            body: None,
        })
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn method(&self) -> &str {
        "GET"
    }
    pub fn headers(&self) -> HostHandle {
        self.headers.clone()
    }
    pub fn snapshot(&self) -> Result<BufferedRequest> {
        let headers = self
            .headers
            .downcast_ref::<RefCell<HeaderList>>()
            .ok_or("invalid Request header resource")?
            .borrow()
            .clone();
        Ok(BufferedRequest {
            url: self.url.clone(),
            method: self.method().into(),
            headers,
            body: self.body.clone(),
        })
    }
}

/// Request declarations and runtime handlers are registered together.
pub fn register(hosts: &mut crate::Hosts) -> Result<()> {
    use crate::{HostOp, HostReply, HostType, HostValue};
    let request = HostType::Handle("Request".into());
    hosts.register(
        HostOp::new(
            "Request",
            vec![HostType::String],
            request.clone(),
            false,
            |args| {
                let HostValue::String(input) = &args[0] else {
                    unreachable!("checked Request input")
                };
                HostReply::Ready(
                    RequestObject::new(input)
                        .map(|value| HostValue::Handle(HostHandle::new("Request", value))),
                )
            },
        )
        .with_global_binding()
        .with_result_channel(),
    )?;
    for field in ["url", "method", "headers"] {
        let result = if field == "headers" {
            HostType::Handle("Headers".into())
        } else {
            HostType::String
        };
        hosts.register(
            HostOp::new(
                &format!("__request_{field}"),
                vec![request.clone()],
                result,
                false,
                move |args| {
                    let HostValue::Handle(handle) = &args[0] else {
                        unreachable!("checked Request receiver")
                    };
                    let Some(request) = handle.downcast_ref::<RequestObject>() else {
                        return HostReply::Ready(Err("invalid Request resource".into()));
                    };
                    HostReply::Ready(Ok(match field {
                        "url" => HostValue::String(request.url().into()),
                        "method" => HostValue::String(request.method().into()),
                        "headers" => HostValue::Handle(request.headers()),
                        _ => unreachable!("closed Request property catalog"),
                    }))
                },
            )
            .with_receiver_property("Request", field),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_normalize_urls_keep_fragments_and_have_no_body() {
        let request =
            RequestObject::new("https://EXAMPLE.com:443/a/../tour?q=hello world#details").unwrap();
        assert_eq!(
            request.url(),
            "https://example.com/tour?q=hello%20world#details"
        );
        assert_eq!(request.method(), "GET");
        let data = request.snapshot().unwrap();
        assert_eq!(data.url, request.url());
        assert_eq!(data.method, "GET");
        assert!(data.headers.entries().is_empty());
        assert_eq!(data.body, None);
    }
    #[test]
    fn invalid_inputs_and_credentials_are_rejected_before_exposing_a_resource() {
        for input in [
            "",
            "/relative",
            "://invalid",
            "https://user:secret@example.com/",
            "https://user:@example.com/",
            "https://:secret@example.com/",
        ] {
            assert!(RequestObject::new(input).is_err(), "{input}");
        }
        assert_eq!(
            RequestObject::new("https://@example.com/").unwrap().url(),
            "https://example.com/"
        );
    }
    #[test]
    fn canonical_header_aliases_and_transport_snapshots_have_distinct_ownership() {
        let request = RequestObject::new("https://example.com").unwrap();
        let headers = request.headers();
        assert_eq!(headers, request.headers());
        let list = headers.downcast_ref::<RefCell<HeaderList>>().unwrap();
        list.borrow_mut().append("X-Project", "Deka").unwrap();
        let snapshot = request.snapshot().unwrap();
        list.borrow_mut().set("x-project", "Changed").unwrap();
        assert_eq!(
            snapshot.headers.get("X-Project").unwrap(),
            Some("Deka".into())
        );
        assert_eq!(
            request
                .snapshot()
                .unwrap()
                .headers
                .get("x-project")
                .unwrap(),
            Some("Changed".into())
        );
        drop(request);
        list.borrow_mut().append("alive", "yes").unwrap();
        assert_eq!(list.borrow().get("alive").unwrap(), Some("yes".into()));
    }
    #[test]
    fn separate_requests_do_not_share_headers_and_opaque_urls_are_valid_inputs() {
        let first = RequestObject::new("data:text/plain,hello#part").unwrap();
        let second = RequestObject::new("mailto:friend@example.com").unwrap();
        first
            .headers()
            .downcast_ref::<RefCell<HeaderList>>()
            .unwrap()
            .borrow_mut()
            .set("X-Test", "one")
            .unwrap();
        assert!(second.snapshot().unwrap().headers.entries().is_empty());
        assert_eq!(first.url(), "data:text/plain,hello#part");
        assert_eq!(second.url(), "mailto:friend@example.com");
    }
}
