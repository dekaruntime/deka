//! Wake and turn results contain no window-backend types.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Wake, Waker},
};
#[derive(Debug, Clone, Copy)]
pub struct Turn {
    pub progressed: bool,
    pub instructions: u64,
}
#[derive(Default)]
pub(crate) struct ReadyWake {
    ready: AtomicBool,
    parent: Mutex<Option<Waker>>,
}
impl ReadyWake {
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn parent(&self) -> Waker {
        self.parent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| Waker::noop().clone())
    }
    pub fn clear(&self) {
        self.ready.store(false, Ordering::Release);
    }
    pub fn set_parent(&self, parent: &Waker) {
        *self.parent.lock().unwrap_or_else(|e| e.into_inner()) = Some(parent.clone());
    }
}
impl Wake for ReadyWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.ready.store(true, Ordering::Release);
        let parent = self
            .parent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(parent) = parent {
            parent.wake();
        }
    }
}
