//! Rust-owned abort state. Subscribers belong to pending operations, not detached tasks.
use crate::{HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::{rc::Rc, time::Duration};
use tokio::{sync::watch, time::Instant};

struct SignalState {
    reason: watch::Sender<Option<String>>,
    deadline: Option<Instant>,
}
#[derive(Clone)]
pub struct AbortSignalObject(Rc<SignalState>);
impl Default for AbortSignalObject {
    fn default() -> Self {
        Self::new(None)
    }
}
impl AbortSignalObject {
    fn new(deadline: Option<Instant>) -> Self {
        Self(Rc::new(SignalState {
            reason: watch::channel(None).0,
            deadline,
        }))
    }
    pub fn timeout(ms: f64) -> Result<Self> {
        if !ms.is_finite() || ms < 0. || ms.fract() != 0. || ms > ((1u64 << 53) - 1) as f64 {
            return Err("timeout must be a nonnegative safe integer in milliseconds".into());
        }
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(ms as u64))
            .ok_or("timeout deadline is out of range")?;
        Ok(Self::new(Some(deadline)))
    }
    pub fn abort(&self, reason: String) {
        // First abort wins, including the first deadline observed by any alias.
        self.refresh();
        self.set_reason(reason);
    }
    fn set_reason(&self, reason: String) {
        self.0.reason.send_if_modified(|stored| {
            if stored.is_some() {
                false
            } else {
                *stored = Some(reason);
                true
            }
        });
    }
    fn refresh(&self) {
        if self
            .0
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.set_reason("TimeoutError".into());
        }
    }
    pub fn reason(&self) -> Option<String> {
        self.refresh();
        self.0.reason.borrow().clone()
    }
    pub fn handle(&self) -> HostHandle {
        HostHandle::new("AbortSignal", self.clone())
    }
    pub async fn cancelled(&self) -> String {
        let mut subscriber = self.0.reason.subscribe();
        self.refresh();
        loop {
            if let Some(reason) = subscriber.borrow().clone() {
                return reason;
            }
            if let Some(deadline) = self.0.deadline {
                tokio::select! {
                    changed = subscriber.changed() => { changed.expect("signal retains its sender"); },
                    () = tokio::time::sleep_until(deadline) => self.refresh(),
                }
            } else {
                subscriber
                    .changed()
                    .await
                    .expect("signal retains its sender");
            }
        }
    }
}
struct AbortControllerObject {
    signal: AbortSignalObject,
    handle: HostHandle,
}
impl AbortControllerObject {
    fn new() -> Self {
        let signal = AbortSignalObject::default();
        let handle = signal.handle();
        Self { signal, handle }
    }
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    let controller = HostType::Handle("AbortController".into());
    let signal = HostType::Handle("AbortSignal".into());
    hosts.register(
        HostOp::new("AbortController", vec![], controller.clone(), false, |_| {
            HostReply::Ready(Ok(HostValue::Handle(HostHandle::new(
                "AbortController",
                AbortControllerObject::new(),
            ))))
        })
        .with_global_binding(),
    )?;
    hosts.register(
        HostOp::new(
            "__abort_controller_signal",
            vec![controller.clone()],
            signal.clone(),
            false,
            |args| {
                let HostValue::Handle(handle) = &args[0] else {
                    unreachable!("checked controller")
                };
                let value = handle
                    .downcast_ref::<AbortControllerObject>()
                    .ok_or("invalid AbortController resource");
                HostReply::Ready(
                    value
                        .map(|value| HostValue::Handle(value.handle.clone()))
                        .map_err(str::to_owned),
                )
            },
        )
        .with_receiver_property("AbortController", "signal"),
    )?;
    hosts.register(
        HostOp::new(
            "__abort_controller_abort",
            vec![controller, HostType::String],
            HostType::Unit,
            false,
            |args| {
                let [HostValue::Handle(handle), HostValue::String(reason)] = args.as_slice() else {
                    unreachable!("checked abort")
                };
                let value = handle
                    .downcast_ref::<AbortControllerObject>()
                    .ok_or("invalid AbortController resource");
                HostReply::Ready(
                    value
                        .map(|value| {
                            value.signal.abort(reason.clone());
                            HostValue::Unit
                        })
                        .map_err(str::to_owned),
                )
            },
        )
        .with_defaults(vec![HostValue::String("AbortError".into())])
        .with_receiver_method("AbortController", "abort"),
    )?;
    for field in ["aborted", "reason"] {
        let output = if field == "aborted" {
            HostType::Bool
        } else {
            HostType::Option(Box::new(HostType::String))
        };
        hosts.register(
            HostOp::new(
                &format!("__abort_signal_{field}"),
                vec![signal.clone()],
                output,
                false,
                move |args| {
                    let HostValue::Handle(handle) = &args[0] else {
                        unreachable!("checked signal")
                    };
                    let result = handle
                        .downcast_ref::<AbortSignalObject>()
                        .ok_or("invalid AbortSignal resource")
                        .map(|value| match field {
                            "aborted" => HostValue::Bool(value.reason().is_some()),
                            "reason" => HostValue::Option(
                                value
                                    .reason()
                                    .map(|reason| Box::new(HostValue::String(reason))),
                            ),
                            _ => unreachable!("closed abort property catalog"),
                        })
                        .map_err(str::to_owned);
                    HostReply::Ready(result)
                },
            )
            .with_receiver_property("AbortSignal", field),
        )?;
    }
    hosts.register(
        HostOp::new(
            "__abort_signal_timeout",
            vec![HostType::Number],
            signal,
            false,
            |args| {
                let HostValue::Number(ms) = args[0] else {
                    unreachable!("checked timeout")
                };
                HostReply::Ready(
                    AbortSignalObject::timeout(ms).map(|value| HostValue::Handle(value.handle())),
                )
            },
        )
        .with_namespace_binding("AbortSignal", "timeout")
        .with_result_channel(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn deadlines_use_the_runtime_clock_and_aliases_keep_the_first_reason() {
        let signal = AbortSignalObject::timeout(100.).unwrap();
        let alias = signal.clone();
        assert_eq!(signal.reason(), None);
        tokio::time::advance(Duration::from_millis(99)).await;
        assert_eq!(alias.reason(), None);
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(signal.cancelled().await, "TimeoutError");
        alias.abort("later".into());
        assert_eq!(signal.reason().as_deref(), Some("TimeoutError"));
        let early = AbortSignalObject::timeout(100.).unwrap();
        early.abort("first".into());
        tokio::time::advance(Duration::from_millis(100)).await;
        assert_eq!(early.cancelled().await, "first");
    }
    #[test]
    fn invalid_durations_are_results_without_a_reactor() {
        for ms in [f64::NAN, f64::INFINITY, -1., 0.5, (1u64 << 53) as f64] {
            assert!(AbortSignalObject::timeout(ms).is_err());
        }
    }
    #[tokio::test]
    async fn cancellation_wakes_all_waiters_and_drops_each_subscription() {
        let signal = AbortSignalObject::default();
        let a = signal.cancelled();
        let b = signal.cancelled();
        tokio::pin!(a, b);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(a.as_mut(), cx).is_pending());
            assert!(std::future::Future::poll(b.as_mut(), cx).is_pending());
            assert_eq!(signal.0.reason.receiver_count(), 2);
            signal.abort("stop".into());
            std::task::Poll::Ready(())
        })
        .await;
        assert_eq!(a.await, "stop");
        assert_eq!(b.await, "stop");
        assert_eq!(signal.0.reason.receiver_count(), 0);
    }
}
