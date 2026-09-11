//! `deka dev` build-slot invalidation wiring (deka#725).
//!
//! The notify watcher in `serve` reports local filesystem changes here; this
//! module maps them onto the build manifest's recorded observations and asks
//! the registered refresh callback to rematerialize exactly the affected
//! slots. The callback lives in `cli`, which owns the dsc orchestration; the
//! runtime only decides *which* slots changed and logs the decision.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use runtime_core::framework::{
    BuildManifest, SlotInvalidation, affected_slots, compiler_cache_dir_with,
};

/// What one watch event asks the refresh hook to do.
#[derive(Debug, Clone, Default)]
pub struct BuildSlotRefreshRequest {
    /// Observation-matched slot ids whose source files did not change (their
    /// compiler ids are still current).
    pub slots: Vec<String>,
    /// Source files (project-relative spelling) whose own content changed.
    /// Slot ids embed the compiler span, so an edit above a `build {}` block
    /// changes its id and the manifest's ids for these files are stale: the
    /// hook must replan the file and rematerialize every slot it NOW
    /// declares, replacing the file's manifest slots wholesale (Codex review
    /// of deka#729).
    pub replan_files: Vec<String>,
}

impl BuildSlotRefreshRequest {
    /// Every planned slot: the watcher could not prove relevance (no build
    /// manifest). Implementations must treat it as a deliberate, logged
    /// coarse rebuild.
    pub fn coarse() -> Self {
        Self::default()
    }

    /// Both lists empty: the watcher could not prove relevance (no build
    /// manifest) — implementations must rematerialize every planned slot as
    /// a deliberate, logged coarse rebuild.
    pub fn is_coarse(&self) -> bool {
        self.slots.is_empty() && self.replan_files.is_empty()
    }
}

/// Rematerializes build slots for `project_root` per the request.
pub type BuildSlotRefresh =
    Arc<dyn Fn(&Path, BuildSlotRefreshRequest) -> Result<(), String> + Send + Sync + 'static>;

fn refresh_hook() -> &'static Mutex<Option<BuildSlotRefresh>> {
    static HOOK: OnceLock<Mutex<Option<BuildSlotRefresh>>> = OnceLock::new();
    HOOK.get_or_init(|| Mutex::new(None))
}

/// Install the refresh callback. Called once by the CLI before `serve`;
/// without a hook, watch events only log the invalidation decision.
pub fn set_build_slot_refresh(refresh: BuildSlotRefresh) {
    if let Ok(mut hook) = refresh_hook().lock() {
        *hook = Some(refresh);
    }
}

/// Handle one batch of changed local paths in a dev watch session.
/// `project_root` is the app-router project root; `changed` holds the
/// watcher's normalized (forward-slash) paths. Returns true when a refresh
/// hook ran and rematerialized at least one slot, so the caller can drop
/// caches that key on the module graph (which does not cover build values);
/// the caller is expected to evict the engine's shared pool — those
/// isolates back the `deka:dev/*` value modules.
pub fn on_watch_event(project_root: &Path, changed: &[String], dev_mode: bool) -> bool {
    if !dev_mode {
        return false;
    }
    let hook = match refresh_hook().lock() {
        Ok(hook) => hook.clone(),
        Err(_) => return false,
    };
    let Some(hook) = hook else {
        return false;
    };

    let manifest_path = compiler_cache_dir_with(project_root, dev_mode).join("build-manifest.json");
    let manifest = match std::fs::read_to_string(&manifest_path) {
        Ok(raw) => match serde_json::from_str::<BuildManifest>(&raw) {
            Ok(manifest) => Some(manifest),
            Err(err) => {
                stdio::log(
                    "watch",
                    &format!(
                        "build manifest at {} is unreadable ({err}); cannot map the change to build slots",
                        manifest_path.display()
                    ),
                );
                None
            }
        },
        Err(_) => {
            stdio::log(
                "watch",
                &format!(
                    "no build manifest at {}; cannot map the change to build slots",
                    manifest_path.display()
                ),
            );
            None
        }
    };

    let invalidation = match &manifest {
        Some(manifest) => {
            let changed: Vec<PathBuf> = changed.iter().map(PathBuf::from).collect();
            affected_slots(manifest, project_root, &changed)
        }
        // Deliberate coarse fallback: without a manifest, no relevance proof
        // exists, so every planned slot must rematerialize. Logged, not
        // silent.
        None => SlotInvalidation {
            slots: Default::default(),
            source_files: Default::default(),
            coarse: true,
        },
    };

    let request = BuildSlotRefreshRequest {
        slots: invalidation.slots.iter().cloned().collect(),
        replan_files: invalidation.source_files.iter().cloned().collect(),
    };
    if invalidation.coarse {
        stdio::log(
            "watch",
            &format!(
                "coarse build invalidation: rematerializing all build slots ({})",
                changed.join(", ")
            ),
        );
    } else if request.slots.is_empty() && request.replan_files.is_empty() {
        stdio::log(
            "watch",
            &format!("no build slots affected by {}", changed.join(", ")),
        );
        return false;
    } else {
        if !request.slots.is_empty() {
            stdio::log(
                "watch",
                &format!(
                    "invalidating build slots [{}]: observed input changed ({})",
                    request.slots.join(", "),
                    changed.join(", ")
                ),
            );
        }
        if !request.replan_files.is_empty() {
            stdio::log(
                "watch",
                &format!(
                    "replanning build slots in [{}]: source file changed ({})",
                    request.replan_files.join(", "),
                    changed.join(", ")
                ),
            );
        }
    }

    // The refresh rematerializes build slots via `block_on` on its own
    // current-thread runtime; the watch task runs on the dev server's tokio
    // runtime, so drive the hook from a plain thread (and join it — the
    // watch loop can wait; serving happens on other tasks).
    let outcome = std::thread::scope(|scope| {
        scope
            .spawn(|| hook(project_root, request))
            .join()
            .map_err(|_| "build slot refresh panicked".to_string())
    });
    match outcome {
        Ok(Ok(())) => {
            stdio::log(
                "watch",
                "rematerialized build slots; re-render on next request",
            );
            true
        }
        Ok(Err(err)) => {
            stdio::log(
                "watch",
                &format!("failed to rematerialize build slots: {err}"),
            );
            false
        }
        Err(err) => {
            stdio::log("watch", &format!("build slot refresh panicked: {err}"));
            false
        }
    }
}
