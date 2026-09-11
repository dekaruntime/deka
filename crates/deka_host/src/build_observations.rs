//! Per-build-slot filesystem observation recording (deka#725).
//!
//! While the host materializes compiler-planned `build {}` values, every
//! permitted local filesystem read performed through the fs bridge is
//! recorded against the active build slot. `deka dev` later uses those
//! observations for targeted invalidation of materialized values.
//!
//! The registry is a process-global stack, not a thread-local: fs bridge ops
//! run on the tokio blocking pool (deka#578), so observations for one slot can
//! arrive from several threads. (The policy itself is NOT process-global since
//! deka#801 — it travels per execution via the security context; only this
//! observation registry is.) Outside build execution the stack is empty and
//! recording is a no-op.
//!
//! Known v1 limitation: in a long-lived `deka dev` process, an fs bridge call
//! made by concurrently served runtime code while a build slot is active is
//! recorded into that slot's list. The effect is conservative extra
//! rematerialization, never wrong values; per-request tagging is follow-up.

use std::sync::{Mutex, OnceLock};

use runtime_core::framework::{FsObservation, FsObservationKind};

struct ActiveSlot {
    slot_id: String,
    observations: Vec<FsObservation>,
}

fn active_slots() -> &'static Mutex<Vec<ActiveSlot>> {
    static ACTIVE: OnceLock<Mutex<Vec<ActiveSlot>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Begin recording observations for `slot_id`. Must be paired with exactly
/// one [`end_build_slot`] (even on error); slots execute sequentially, so the
/// stack discipline is a simple push/pop.
pub fn begin_build_slot(slot_id: &str) {
    if let Ok(mut slots) = active_slots().lock() {
        slots.push(ActiveSlot {
            slot_id: slot_id.to_string(),
            observations: Vec::new(),
        });
    }
}

/// Stop recording and return `(slot_id, observations)` for the innermost
/// active slot. Sorted and deduped so manifests stay deterministic.
pub fn end_build_slot() -> Option<(String, Vec<FsObservation>)> {
    let mut slots = active_slots().lock().ok()?;
    let mut active = slots.pop()?;
    active.observations.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.kind.cmp(&b.kind)));
    active.observations.dedup();
    Some((active.slot_id, active.observations))
}

/// Record one observation for the innermost active build slot. No-op outside
/// build execution.
pub fn record_build_observation(path: &str, kind: FsObservationKind) {
    if path.is_empty() {
        return;
    }
    if let Ok(mut slots) = active_slots().lock() {
        if let Some(active) = slots.last_mut() {
            active.observations.push(FsObservation {
                path: path.to_string(),
                kind,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests in this module: they drive the same
    /// process-global slot stack, and cargo runs them as parallel threads
    /// in one process, so an interleaved run makes
    /// `end_build_slot().is_none()` observe the other test's open slot.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    #[test]
    fn records_only_while_a_slot_is_active() {
        let _guard = serial();
        record_build_observation("data/a.json", FsObservationKind::Read);
        assert!(end_build_slot().is_none());

        begin_build_slot("slot-1");
        record_build_observation("data/b.json", FsObservationKind::Read);
        record_build_observation("data", FsObservationKind::DirectoryListing);
        let (id, observations) = end_build_slot().expect("slot drained");
        assert_eq!(id, "slot-1");
        assert_eq!(
            observations,
            vec![
                FsObservation {
                    path: "data".to_string(),
                    kind: FsObservationKind::DirectoryListing
                },
                FsObservation {
                    path: "data/b.json".to_string(),
                    kind: FsObservationKind::Read
                },
            ]
        );

        // Drained: nothing leaks into the next slot or runtime calls.
        record_build_observation("data/c.json", FsObservationKind::Read);
        assert!(end_build_slot().is_none());
    }

    #[test]
    fn duplicate_observations_are_deduped() {
        let _guard = serial();
        begin_build_slot("slot-2");
        record_build_observation("data/a.json", FsObservationKind::Read);
        record_build_observation("data/a.json", FsObservationKind::Read);
        let (_, observations) = end_build_slot().expect("slot drained");
        assert_eq!(observations.len(), 1);
    }
}
