//! Rust-owned buffered Response. Body aliases share a single consumption state.
use crate::{HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::cell::RefCell;

use crate::http_body::{Body, HasBody};

/// Network hosts can construct the same object as the language constructor.
/// There is no stream/tee implementation hidden behind this buffered slice.
pub struct ResponseObject {
    status: u16,
    status_text: String,
    url: String,
    headers: HostValue,
    body: Body,
}
impl ResponseObject {
    pub fn from_parts(
        status: u16,
        status_text: String,
        url: String,
        headers: crate::http_headers::HeaderList,
        body: Option<Vec<u8>>,
    ) -> Result<Self> {
        if !(200..=599).contains(&status) {
            return Err("response status must be between 200 and 599".into());
        }
        if status_text
            .chars()
            .any(|c| u32::from(c) > 255 || matches!(c, '\r' | '\n' | '\0'))
        {
            return Err("invalid response status text".into());
        }
        if matches!(status, 204 | 205 | 304) && body.is_some() {
            return Err("response status cannot have a body".into());
        }
        Ok(Self {
            status,
            status_text,
            url,
            headers: crate::http_headers::handle(headers),
            body: Body::new(body),
        })
    }
    pub fn new(text: &str) -> Result<Self> {
        let mut headers = crate::http_headers::HeaderList::default();
        headers.set("content-type", "text/plain;charset=UTF-8")?;
        Self::from_parts(
            200,
            String::new(),
            String::new(),
            headers,
            Some(text.as_bytes().to_vec()),
        )
    }
    pub fn into_host_value(self) -> HostValue {
        HostValue::Handle(HostHandle::new("Response", self))
    }
    pub fn status(&self) -> u16 {
        self.status
    }
    pub fn ok(&self) -> bool {
        (200..=299).contains(&self.status)
    }
    pub fn headers(&self) -> HostValue {
        self.headers.clone()
    }
    pub fn body_used(&self) -> bool {
        self.body.used()
    }
    fn consume(&self) -> Result<Vec<u8>> {
        self.body.consume().map_err(|e| format!("response {e}"))
    }
    pub(crate) fn wire_parts(&self) -> Result<(u16, crate::http_headers::HeaderList, Vec<u8>)> {
        let HostValue::Handle(headers) = &self.headers else {
            return Err("invalid Response headers".into());
        };
        let headers = headers
            .downcast_ref::<RefCell<crate::http_headers::HeaderList>>()
            .ok_or("invalid Response header resource")?
            .borrow()
            .clone();
        Ok((self.status, headers, self.consume()?))
    }
}
impl HasBody for ResponseObject {
    const BRAND: &'static str = "Response";
    fn body(&self) -> &Body {
        &self.body
    }
}

pub fn register(hosts: &mut Hosts) -> Result<()> {
    hosts.register(
        HostOp::new(
            "Response",
            vec![HostType::String],
            HostType::Handle("Response".into()),
            false,
            |args| {
                let HostValue::String(text) = &args[0] else {
                    unreachable!("checked response body")
                };
                HostReply::Ready(ResponseObject::new(text).map(ResponseObject::into_host_value))
            },
        )
        .with_global_binding()
        .with_result_channel(),
    )?;
    for (property, ty) in [
        ("status", HostType::Number),
        ("ok", HostType::Bool),
        ("statusText", HostType::String),
        ("url", HostType::String),
        ("headers", HostType::Handle("Headers".into())),
    ] {
        hosts.register(
            HostOp::new(
                &format!("__response_{property}"),
                vec![HostType::Handle("Response".into())],
                ty,
                false,
                move |args| {
                    let HostValue::Handle(handle) = &args[0] else {
                        unreachable!("checked Response")
                    };
                    let Some(response) = handle.downcast_ref::<ResponseObject>() else {
                        return HostReply::Ready(Err("invalid Response resource".into()));
                    };
                    HostReply::Ready(Ok(match property {
                        "status" => HostValue::Number(f64::from(response.status())),
                        "ok" => HostValue::Bool(response.ok()),
                        "statusText" => HostValue::String(response.status_text.clone()),
                        "url" => HostValue::String(response.url.clone()),
                        "headers" => response.headers(),
                        "bodyUsed" => HostValue::Bool(response.body_used()),
                        _ => unreachable!("registered Response property"),
                    }))
                },
            )
            .with_receiver_property("Response", property),
        )?;
    }
    crate::http_body::register::<ResponseObject>(hosts, "response")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_headers::HeaderList;

    #[test]
    fn status_validation_and_metadata_use_the_same_resource_as_network_hosts() {
        for status in [200, 201, 299, 300, 400, 599] {
            let response = ResponseObject::from_parts(
                status,
                "done".into(),
                "https://example.test/".into(),
                HeaderList::default(),
                None,
            )
            .unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(response.ok(), status < 300);
            assert_eq!(response.url, "https://example.test/");
        }
        for status in [0, 100, 199, 600] {
            assert!(
                ResponseObject::from_parts(
                    status,
                    String::new(),
                    String::new(),
                    HeaderList::default(),
                    None
                )
                .is_err()
            );
        }
        for status in [204, 205, 304] {
            assert!(
                ResponseObject::from_parts(
                    status,
                    String::new(),
                    String::new(),
                    HeaderList::default(),
                    Some(vec![])
                )
                .is_err()
            );
        }
        for status_text in ["a\r\nb", "\0", "🙂"] {
            assert!(
                ResponseObject::from_parts(
                    200,
                    status_text.into(),
                    String::new(),
                    HeaderList::default(),
                    None
                )
                .is_err()
            );
        }
    }
    #[test]
    fn absent_and_present_empty_bodies_have_different_consumption_states() {
        let absent = ResponseObject::from_parts(
            204,
            String::new(),
            String::new(),
            HeaderList::default(),
            None,
        )
        .unwrap();
        for _ in 0..2 {
            assert_eq!(absent.body.text().unwrap(), "");
            assert!(!absent.body_used());
        }
        let empty = ResponseObject::new("").unwrap();
        assert!(!empty.body_used());
        assert!(empty.consume().unwrap().is_empty());
        assert!(empty.body_used());
        assert!(empty.body.text().is_err());
    }
    #[test]
    fn utf8_body_reads_remove_bom_and_replace_malformed_bytes_without_changing_bytes() {
        let bytes = vec![0xef, 0xbb, 0xbf, b'A', 0xff];
        let text = ResponseObject::from_parts(
            200,
            String::new(),
            String::new(),
            HeaderList::default(),
            Some(bytes.clone()),
        )
        .unwrap();
        assert_eq!(text.body.text().unwrap(), "A�");
        assert!(text.consume().is_err());
        let raw = ResponseObject::from_parts(
            200,
            String::new(),
            String::new(),
            HeaderList::default(),
            Some(bytes.clone()),
        )
        .unwrap();
        assert_eq!(raw.consume().unwrap(), bytes);
    }
    #[test]
    fn aliases_share_body_and_headers_and_survive_the_original_owner() {
        let original = ResponseObject::new("Deka").unwrap().into_host_value();
        let alias = original.clone();
        let HostValue::Handle(handle) = &original else {
            panic!("Response handle")
        };
        let response = handle.downcast_ref::<ResponseObject>().unwrap();
        let header = response.headers();
        assert_eq!(header, response.headers());
        let HostValue::Handle(h) = &header else {
            panic!("Headers handle")
        };
        h.downcast_ref::<RefCell<HeaderList>>()
            .unwrap()
            .borrow_mut()
            .set("X", "After")
            .unwrap();
        drop(original);
        let HostValue::Handle(h) = &alias else {
            panic!("Response alias")
        };
        let response = h.downcast_ref::<ResponseObject>().unwrap();
        assert_eq!(response.body.text().unwrap(), "Deka");
        assert!(response.body_used());
        assert!(response.body.text().is_err());
        assert_eq!(
            h.downcast_ref::<ResponseObject>().unwrap().headers(),
            header
        );
        drop(alias);
        let HostValue::Handle(h) = header else {
            panic!("Headers lifetime")
        };
        assert_eq!(
            h.downcast_ref::<RefCell<HeaderList>>()
                .unwrap()
                .borrow()
                .get("x")
                .unwrap(),
            Some("After".into())
        );
    }
}
