//! Familiar timer globals implemented with VM-owned Rust work.
use crate::*;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

struct Timer {
    wait: HostFuture,
    callback: HostCallback,
    job: HostJob,
    ms: f64,
    repeat: bool,
}
impl Future for Timer {
    type Output = Result<HostValue>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.job.is_cancelled() {
            return Poll::Ready(Ok(HostValue::Unit));
        }
        match self.wait.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                result?;
                self.callback.call_for(&self.job)?;
                if self.repeat {
                    self.wait = crate::time::sleep(self.ms)?;
                    // At most one callback per poll, including zero intervals.
                    // The next turn installs the replacement timer's waker.
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(Ok(HostValue::Unit))
                }
            }
        }
    }
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    for (name, repeat) in [("setTimeout", false), ("setInterval", true)] {
        hosts.register(
            HostOp::with_context(
                name,
                vec![HostType::Callback, HostType::Number],
                HostType::Number,
                false,
                None,
                move |context, args| {
                    let [HostValue::Callback(callback), HostValue::Number(ms)] = args.as_slice()
                    else {
                        unreachable!()
                    };
                    let result = (|| {
                        let wait = crate::time::sleep(*ms)?;
                        let job = context.job()?;
                        let id = job.id();
                        context.spawn(
                            job.clone(),
                            Box::pin(Timer {
                                wait,
                                callback: callback.clone(),
                                job,
                                ms: *ms,
                                repeat,
                            }),
                        )?;
                        Ok(HostValue::Number(id as f64))
                    })();
                    HostReply::Ready(result)
                },
            )
            .with_global_binding(),
        )?;
    }
    for name in ["clearTimeout", "clearInterval"] {
        hosts.register(
            HostOp::with_context(
                name,
                vec![HostType::Number],
                HostType::Unit,
                false,
                None,
                |context, args| {
                    let HostValue::Number(id) = args[0] else {
                        unreachable!()
                    };
                    let result = if id.is_finite()
                        && id >= 1.
                        && id <= (1u64 << 53) as f64
                        && id.fract() == 0.
                    {
                        context.cancel(id as u64).map(|_| HostValue::Unit)
                    } else {
                        Ok(HostValue::Unit)
                    };
                    HostReply::Ready(result)
                },
            )
            .with_global_binding(),
        )?;
    }
    Ok(())
}
