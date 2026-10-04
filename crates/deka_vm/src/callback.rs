//! Host work enters its owning VM through a queue, never by re-entering it.
use crate::{HostFuture, HostType, HostValue, Result, heap::Handle};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    fmt,
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context, Poll, Waker},
};

struct State {
    alive: bool,
    next_job: u64,
    jobs: BTreeMap<u64, Weak<Job>>,
    callbacks: Vec<Weak<Callback>>,
    queue: VecDeque<Command>,
    waker: Option<Waker>,
}
#[derive(Clone)]
pub struct HostContext(Rc<RefCell<State>>);
impl Default for HostContext {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(State {
            alive: true,
            next_job: 0,
            jobs: BTreeMap::new(),
            callbacks: vec![],
            queue: VecDeque::new(),
            waker: None,
        })))
    }
}
struct Callback {
    handle: Handle,
    owner: Weak<RefCell<State>>,
}
/// A rooted language callback. Clones retain it; dropping the last clone
/// releases its root at the next VM collection.
#[derive(Clone)]
pub struct HostCallback(Rc<Callback>, Option<Rc<Signature>>);
struct Signature {
    args: Vec<HostType>,
    result: HostType,
    result_channel: bool,
}
struct CompletionState {
    result: Option<Result<HostValue>>,
    waker: Option<Waker>,
}
/// Owned by queued/running work; dropping it resolves an abandoned invocation.
pub(crate) struct Completion {
    state: Rc<RefCell<CompletionState>>,
    pub(crate) result: HostType,
    pub(crate) result_channel: bool,
}
impl Completion {
    pub(crate) fn resolve(self, result: Result<HostValue>) {
        self.finish(result);
    }
    fn finish(&self, result: Result<HostValue>) {
        let waker = {
            let mut state = self.state.borrow_mut();
            if state.result.is_some() {
                return;
            }
            state.result = Some(result);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.finish(Err("VM cancelled".into()));
    }
}
struct CompletionFuture(Rc<RefCell<CompletionState>>);
impl Future for CompletionFuture {
    type Output = Result<HostValue>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.borrow_mut();
        if let Some(result) = state.result.take() {
            Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
impl fmt::Debug for HostCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HostCallback")
    }
}
impl PartialEq for HostCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl HostCallback {
    /// Enqueue a call for the next VM turn. This never executes DekaScript.
    pub fn call(&self, args: Vec<HostValue>) -> Result<()> {
        self.enqueue(args, None, None)
    }
    #[cfg(feature = "host")]
    pub(crate) fn call_for(&self, job: &HostJob) -> Result<()> {
        self.enqueue(vec![], Some(job.clone()), None)
    }
    pub(crate) fn with_signature(
        mut self,
        args: Vec<HostType>,
        result: HostType,
        result_channel: bool,
    ) -> Self {
        self.1 = Some(Rc::new(Signature {
            args,
            result,
            result_channel,
        }));
        self
    }
    /// Enqueue a typed callback and await its actual return, including its awaits.
    /// Calling this only queues work; the owning VM drives it in bounded turns.
    pub fn call_async(&self, args: Vec<HostValue>) -> Result<HostFuture> {
        let signature = self
            .1
            .as_ref()
            .ok_or("awaitable callback requires a typed signature")?;
        let state = Rc::new(RefCell::new(CompletionState {
            result: None,
            waker: None,
        }));
        self.enqueue(
            args,
            None,
            Some(Completion {
                state: state.clone(),
                result: signature.result.clone(),
                result_channel: signature.result_channel,
            }),
        )?;
        Ok(Box::pin(CompletionFuture(state)))
    }
    fn enqueue(
        &self,
        mut args: Vec<HostValue>,
        job: Option<HostJob>,
        completion: Option<Completion>,
    ) -> Result<()> {
        if let Some(signature) = &self.1
            && (args.len() != signature.args.len()
                || !signature
                    .args
                    .iter()
                    .zip(&args)
                    .all(|(ty, value)| ty.accepts(value)))
        {
            return Err("invalid host callback arguments".into());
        }
        if let Some(signature) = &self.1 {
            args = signature
                .args
                .iter()
                .zip(args)
                .map(|(ty, value)| ty.normalize(value))
                .collect();
        }
        let owner = self.0.owner.upgrade().ok_or("VM cancelled")?;
        HostContext(owner).enqueue(Command::Call {
            callback: self.clone(),
            args,
            job,
            completion,
        })
    }
    pub(crate) fn handle(&self) -> Handle {
        self.0.handle
    }
}
struct Job {
    id: u64,
    cancelled: Cell<bool>,
    owner: Weak<RefCell<State>>,
}
#[derive(Clone)]
pub struct HostJob(Rc<Job>);
impl HostJob {
    pub fn id(&self) -> u64 {
        self.0.id
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.get()
    }
}
pub(crate) enum Command {
    Spawn {
        job: HostJob,
        future: HostFuture,
    },
    Cancel(u64),
    Call {
        callback: HostCallback,
        args: Vec<HostValue>,
        job: Option<HostJob>,
        completion: Option<Completion>,
    },
}
impl HostContext {
    /// Reserve a cancellable host job; scheduling makes it VM-owned work.
    pub fn job(&self) -> Result<HostJob> {
        let mut state = self.0.borrow_mut();
        if !state.alive {
            return Err("VM cancelled".into());
        }
        if state.next_job >= (1 << 53) {
            return Err("host job identifiers exhausted".into());
        }
        state.next_job += 1;
        let job = HostJob(Rc::new(Job {
            id: state.next_job,
            cancelled: Cell::new(false),
            owner: Rc::downgrade(&self.0),
        }));
        state.jobs.insert(job.id(), Rc::downgrade(&job.0));
        Ok(job)
    }
    pub fn spawn(&self, job: HostJob, future: HostFuture) -> Result<()> {
        if !Weak::ptr_eq(&job.0.owner, &Rc::downgrade(&self.0)) {
            return Err("host job belongs to another VM".into());
        }
        self.enqueue(Command::Spawn { job, future })
    }
    /// Unknown or completed job identifiers are harmless.
    pub fn cancel(&self, id: u64) -> Result<()> {
        {
            let state = self.0.borrow();
            if !state.alive {
                return Err("VM cancelled".into());
            }
            if let Some(job) = state.jobs.get(&id).and_then(Weak::upgrade) {
                job.cancelled.set(true);
            }
        }
        self.enqueue(Command::Cancel(id))
    }
    fn enqueue(&self, command: Command) -> Result<()> {
        let waker = {
            let mut state = self.0.borrow_mut();
            if !state.alive {
                return Err("VM cancelled".into());
            }
            state.queue.push_back(command);
            state.waker.clone()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }
    pub(crate) fn set_waker(&self, waker: &Waker) {
        self.0.borrow_mut().waker = Some(waker.clone());
    }
    pub(crate) fn take(&self, limit: usize) -> VecDeque<Command> {
        let mut state = self.0.borrow_mut();
        let count = limit.min(state.queue.len());
        state.queue.drain(..count).collect()
    }
    pub(crate) fn has_commands(&self) -> bool {
        !self.0.borrow().queue.is_empty()
    }
    pub(crate) fn callback(&self, handle: Handle) -> HostCallback {
        let callback = HostCallback(
            Rc::new(Callback {
                handle,
                owner: Rc::downgrade(&self.0),
            }),
            None,
        );
        self.0
            .borrow_mut()
            .callbacks
            .push(Rc::downgrade(&callback.0));
        callback
    }
    pub(crate) fn owns(&self, callback: &HostCallback) -> bool {
        Weak::ptr_eq(&callback.0.owner, &Rc::downgrade(&self.0))
    }
    pub(crate) fn roots(&self) -> Vec<Handle> {
        let mut state = self.0.borrow_mut();
        state.jobs.retain(|_, job| job.strong_count() > 0);
        state
            .callbacks
            .retain(|callback| callback.strong_count() > 0);
        state
            .callbacks
            .iter()
            .filter_map(Weak::upgrade)
            .map(|c| c.handle)
            .collect()
    }
    pub(crate) fn close(&self) {
        // Drop futures/callbacks outside the RefCell borrow: a host future's
        // destructor may try to enqueue work and must receive a closed error.
        let queue = {
            let mut state = self.0.borrow_mut();
            state.alive = false;
            for job in state.jobs.values().filter_map(Weak::upgrade) {
                job.cancelled.set(true);
            }
            state.jobs.clear();
            state.callbacks.clear();
            state.waker = None;
            std::mem::take(&mut state.queue)
        };
        drop(queue);
    }
}
