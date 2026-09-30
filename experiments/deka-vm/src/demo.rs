//! Small real host adapter. This is not a replacement for Deka's production catalog.
use crate::*;
use std::{cell::RefCell, rc::Rc, time::Duration};
pub type Output = Rc<RefCell<Vec<String>>>;
pub fn hosts() -> Result<(Hosts, Output)> {
    let output = Output::default();
    let mut hosts = Hosts::default();
    let sink = output.clone();
    hosts.register(HostOp::new(
        "print",
        vec![HostType::String],
        HostType::Unit,
        false,
        None,
        move |args| {
            let HostValue::String(s) = &args[0] else {
                unreachable!()
            };
            sink.borrow_mut().push(s.clone());
            HostReply::Ready(Ok(HostValue::Unit))
        },
    ))?;
    hosts.register(HostOp::new(
        "sum",
        vec![HostType::Number, HostType::Number],
        HostType::Number,
        false,
        None,
        |args| {
            let [HostValue::Number(a), HostValue::Number(b)] = args.as_slice() else {
                unreachable!()
            };
            HostReply::Ready(Ok(HostValue::Number(a + b)))
        },
    ))?;
    hosts.register(HostOp::new(
        "delay",
        vec![HostType::Number, HostType::String],
        HostType::String,
        true,
        Some("timer"),
        |args| {
            let [HostValue::Number(ms), HostValue::String(value)] = args.as_slice() else {
                unreachable!()
            };
            if !ms.is_finite() || *ms < 0. || *ms > 60_000. {
                return HostReply::Ready(Err("delay must be 0..60000 milliseconds".into()));
            }
            let duration = Duration::from_secs_f64(ms / 1000.);
            let value = value.clone();
            HostReply::Pending(Box::pin(async move {
                tokio::time::sleep(duration).await;
                Ok(HostValue::String(value))
            }))
        },
    ))?;
    Ok((hosts, output))
}
