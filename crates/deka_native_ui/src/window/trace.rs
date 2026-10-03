//! Start-up marks: when each step of opening the window and drawing frame one
//! happened. Recorded once per process (a few dozen entries), read by the
//! measurements; nothing in the window depends on them.
use std::sync::Mutex;
use std::time::Instant;

static MARKS: Mutex<Vec<(&'static str, Instant)>> = Mutex::new(Vec::new());

/// Record `label` now.
pub fn mark(label: &'static str) {
    if let Ok(mut marks) = MARKS.lock() {
        marks.push((label, Instant::now()));
    }
}

/// The marks recorded so far, oldest first.
pub fn marks() -> Vec<(&'static str, Instant)> {
    MARKS.lock().map(|m| m.clone()).unwrap_or_default()
}
