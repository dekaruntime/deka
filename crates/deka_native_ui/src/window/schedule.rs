//! When the window draws: only when something changed, every display refresh
//! while a scene animates, and never while the window cannot be seen.
use std::time::{Duration, Instant};

/// How long to wait before trying again when the surface reports the window
/// hidden without a visibility event (display asleep, first frame before the
/// window is composited). A retry, not a spin.
pub(crate) const OCCLUDED_RETRY: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub(crate) struct Schedule {
    /// Something changed since the last presented frame.
    dirty: bool,
    /// The last presented scene was animating.
    animating: bool,
    /// The window is hidden (minimised, covered, other space, display asleep).
    occluded: bool,
    /// The next background turn (live reload polling, VM work).
    next_turn: Option<Instant>,
    /// When to retry a frame the surface refused while no visibility event came.
    retry: Option<Instant>,
}

/// What the event loop should do once its queue is empty.
#[derive(Debug, PartialEq)]
pub(crate) enum Wait {
    /// Sleep until an event arrives.
    Forever,
    /// Sleep until this instant at the latest.
    Until(Instant),
}

impl Schedule {
    pub(crate) fn new() -> Self {
        Self {
            dirty: true,
            animating: false,
            occluded: false,
            next_turn: None,
            retry: None,
        }
    }

    /// Input, resize, scale, focus or application state changed.
    pub(crate) fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// A frame reached the surface.
    pub(crate) fn presented(&mut self, animating: bool) {
        self.dirty = false;
        self.animating = animating;
        self.retry = None;
    }

    /// The window's visibility changed (winit's `Occluded`).
    pub(crate) fn set_occluded(&mut self, occluded: bool) {
        self.occluded = occluded;
        self.retry = None;
        if !occluded {
            self.dirty = true;
        }
    }

    /// The window became visible showing the frame it was given before it
    /// was shown (macOS frame one): redraw only if something changed since.
    pub(crate) fn shown_with_frame(&mut self) {
        self.occluded = false;
        self.retry = None;
    }

    /// The surface refused a frame because the window is not visible.
    pub(crate) fn surface_occluded(&mut self, now: Instant) {
        self.occluded = true;
        self.retry = Some(now + OCCLUDED_RETRY);
    }

    /// A frame failed or had nothing to draw into; draw again on the next
    /// change instead of retrying a failing frame in a loop.
    pub(crate) fn skipped(&mut self) {
        self.dirty = false;
        self.animating = false;
    }

    pub(crate) fn occluded(&self) -> bool {
        self.occluded
    }

    /// Whether a redraw should be requested now.
    pub(crate) fn wants_frame(&self) -> bool {
        !self.occluded && (self.dirty || self.animating)
    }

    /// Plan the next background turn `interval` from `now` (`None`: no turns).
    pub(crate) fn plan_turn(&mut self, now: Instant, interval: Option<Duration>) {
        self.next_turn = interval.map(|i| now + i);
    }

    /// Whether a background turn is due.
    pub(crate) fn turn_due(&self, now: Instant) -> bool {
        self.next_turn.is_some_and(|t| now >= t)
    }

    /// Whether the occluded retry is due; clears it and lets one frame through.
    pub(crate) fn take_retry(&mut self, now: Instant) -> bool {
        if self.retry.is_some_and(|t| now >= t) {
            self.retry = None;
            self.occluded = false;
            self.dirty = true;
            return true;
        }
        false
    }

    pub(crate) fn wait(&self) -> Wait {
        match (self.next_turn, self.retry) {
            (Some(a), Some(b)) => Wait::Until(a.min(b)),
            (Some(t), None) | (None, Some(t)) => Wait::Until(t),
            (None, None) => Wait::Forever,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idles_after_a_static_frame_and_redraws_once_per_change() {
        let mut s = Schedule::new();
        assert!(s.wants_frame(), "the first frame is always drawn");
        s.presented(false);
        assert!(!s.wants_frame(), "a static scene does not redraw");
        assert_eq!(s.wait(), Wait::Forever, "an idle window sleeps");
        s.invalidate();
        assert!(s.wants_frame());
        s.presented(false);
        assert!(!s.wants_frame());
    }

    #[test]
    fn animates_until_the_scene_stops() {
        let mut s = Schedule::new();
        s.presented(true);
        assert!(s.wants_frame(), "an animating scene draws the next frame");
        s.presented(true);
        assert!(s.wants_frame());
        s.presented(false);
        assert!(!s.wants_frame());
    }

    #[test]
    fn pauses_while_occluded_and_resumes_when_visible() {
        let mut s = Schedule::new();
        s.presented(true);
        s.set_occluded(true);
        assert!(!s.wants_frame(), "no frames while hidden, even animating");
        s.invalidate();
        assert!(!s.wants_frame(), "changes while hidden wait for visibility");
        s.set_occluded(false);
        assert!(s.wants_frame(), "becoming visible redraws");
    }

    #[test]
    fn a_window_shown_with_its_frame_does_not_redraw_it_on_becoming_visible() {
        let mut s = Schedule::new();
        // Frame one went into the hidden window; AppKit then reports it
        // hidden and, once composited, visible.
        s.presented(false);
        s.set_occluded(true);
        s.shown_with_frame();
        assert!(!s.wants_frame(), "the frame on screen is current");
        s.invalidate();
        assert!(s.wants_frame(), "a change still redraws");
    }

    #[test]
    fn retries_a_refused_frame_later_instead_of_spinning() {
        let mut s = Schedule::new();
        let now = Instant::now();
        s.surface_occluded(now);
        assert!(!s.wants_frame());
        assert_eq!(s.wait(), Wait::Until(now + OCCLUDED_RETRY));
        assert!(!s.take_retry(now), "not before the retry time");
        assert!(s.take_retry(now + OCCLUDED_RETRY));
        assert!(s.wants_frame());
    }

    #[test]
    fn a_failing_frame_is_not_retried_until_something_changes() {
        let mut s = Schedule::new();
        s.presented(true);
        s.skipped();
        assert!(
            !s.wants_frame(),
            "no busy loop on a frame that keeps failing"
        );
        s.invalidate();
        assert!(s.wants_frame());
    }

    #[test]
    fn background_turns_wake_the_loop_on_time() {
        let mut s = Schedule::new();
        let now = Instant::now();
        s.plan_turn(now, Some(Duration::from_millis(100)));
        assert_eq!(s.wait(), Wait::Until(now + Duration::from_millis(100)));
        assert!(!s.turn_due(now));
        assert!(s.turn_due(now + Duration::from_millis(100)));
        s.plan_turn(now, None);
        assert_eq!(s.wait(), Wait::Forever);
    }
}
