//! Native time module: wall-clock epoch milliseconds and shared Tokio sleep.
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::time::{SystemTime, UNIX_EPOCH};

fn epoch_milliseconds(time: SystemTime) -> f64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as f64,
        Err(error) => {
            let duration = error.duration();
            // Epoch milliseconds floor toward the earlier instant, including
            // a fractional millisecond before 1970 rather than hiding it as zero.
            let fraction = !duration.subsec_nanos().is_multiple_of(1_000_000);
            -(duration.as_millis() as f64 + f64::from(u8::from(fraction)))
        }
    }
}

pub fn register(hosts: &mut Hosts) -> Result<()> {
    register_with_clock(hosts, SystemTime::now)
}

/// Embedders can supply a deterministic clock; language declarations and timer
/// dispatch are identical to the production clock registration.
pub fn register_with_clock(
    hosts: &mut Hosts,
    clock: impl Fn() -> SystemTime + 'static,
) -> Result<()> {
    crate::time::register(hosts)?;
    hosts.register(HostOp::new(
        "time_now",
        vec![],
        HostType::Number,
        false,
        move |_| HostReply::Ready(Ok(HostValue::Number(epoch_milliseconds(clock())))),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn clock_conversion_is_whole_milliseconds_and_never_zero_fallback() {
        assert_eq!(epoch_milliseconds(UNIX_EPOCH), 0.);
        assert_eq!(
            epoch_milliseconds(UNIX_EPOCH + Duration::from_micros(1500)),
            1.
        );
        assert_eq!(
            epoch_milliseconds(UNIX_EPOCH - Duration::from_micros(1500)),
            -2.
        );
        assert_eq!(
            epoch_milliseconds(UNIX_EPOCH - Duration::from_millis(25)),
            -25.
        );
    }
}
