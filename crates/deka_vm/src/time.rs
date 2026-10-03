//! Native timers shared by production hosts and the VM demo.
use crate::{HostFuture, HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::time::Duration;

/// Construct a timer without requiring a runtime until its future is polled.
/// Invalid durations are argument faults, not operational I/O failures.
pub fn sleep(ms: f64) -> Result<HostFuture> {
    let duration = Duration::try_from_secs_f64(ms / 1000.)
        .map_err(|_| "sleep duration must be finite, nonnegative milliseconds".to_owned())?;
    Ok(Box::pin(async move {
        tokio::time::sleep(duration).await;
        Ok(HostValue::Unit)
    }))
}

pub fn register(hosts: &mut Hosts) -> Result<()> {
    hosts.register(HostOp::new(
        "sleep",
        vec![HostType::Number],
        HostType::Unit,
        true,
        |args| {
            let HostValue::Number(ms) = args[0] else {
                unreachable!()
            };
            match sleep(ms) {
                Ok(future) => HostReply::Pending(future),
                Err(error) => HostReply::Ready(Err(error)),
            }
        },
    ))
}
