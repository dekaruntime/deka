//! Shared buffered body ownership for native Request and Response.
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::cell::RefCell;
#[derive(Clone, Debug)]
enum State {
    Absent,
    Buffered(Vec<u8>),
    Consumed,
}
#[derive(Clone, Debug)]
pub struct Body(RefCell<State>);
impl Body {
    pub fn new(bytes: Option<Vec<u8>>) -> Self {
        Self(RefCell::new(bytes.map_or(State::Absent, State::Buffered)))
    }
    pub fn used(&self) -> bool {
        matches!(*self.0.borrow(), State::Consumed)
    }
    pub fn consume(&self) -> Result<Vec<u8>> {
        let mut state = self.0.borrow_mut();
        match &*state {
            State::Absent => Ok(vec![]),
            State::Consumed => Err("body has already been consumed".into()),
            State::Buffered(_) => {
                let State::Buffered(bytes) = std::mem::replace(&mut *state, State::Consumed) else {
                    unreachable!()
                };
                Ok(bytes)
            }
        }
    }
    pub fn text(&self) -> Result<String> {
        crate::text_codec::decode_utf8(&self.consume()?)
    }
    pub fn snapshot(&self) -> Result<Option<Vec<u8>>> {
        match &*self.0.borrow() {
            State::Absent => Ok(None),
            State::Buffered(b) => Ok(Some(b.clone())),
            State::Consumed => Err("body has already been consumed".into()),
        }
    }
}
pub(crate) trait HasBody: 'static {
    const BRAND: &'static str;
    fn body(&self) -> &Body;
}
pub(crate) fn register<T: HasBody>(hosts: &mut Hosts, prefix: &str) -> Result<()> {
    hosts.register(
        HostOp::new(
            &format!("__{prefix}_bodyUsed"),
            vec![HostType::Handle(T::BRAND.into())],
            HostType::Bool,
            false,
            |args| {
                let HostValue::Handle(h) = &args[0] else {
                    unreachable!("checked body receiver")
                };
                HostReply::Ready(
                    h.downcast_ref::<T>()
                        .ok_or_else(|| "invalid body resource".into())
                        .map(|v| HostValue::Bool(v.body().used())),
                )
            },
        )
        .with_receiver_property(T::BRAND, "bodyUsed"),
    )?;
    for method in ["text", "bytes", "json"] {
        let op = HostOp::new(
            &format!("__{prefix}_{method}"),
            vec![HostType::Handle(T::BRAND.into())],
            if method == "bytes" {
                HostType::Bytes
            } else {
                HostType::String
            },
            true,
            move |args| {
                let HostValue::Handle(h) = &args[0] else {
                    unreachable!("checked body receiver")
                };
                let result = (|| {
                    let v = h.downcast_ref::<T>().ok_or("invalid body resource")?;
                    let result = if method == "bytes" {
                        v.body().consume().map(HostValue::Bytes)
                    } else {
                        v.body().text().map(HostValue::String)
                    };
                    result.map_err(|e| format!("{} {e}", T::BRAND.to_lowercase()))
                })();
                HostReply::Ready(result)
            },
        )
        .with_receiver_method(T::BRAND, method)
        .with_result_channel();
        hosts.register(if method == "json" {
            op.with_json_body()
        } else {
            op
        })?;
    }
    Ok(())
}
