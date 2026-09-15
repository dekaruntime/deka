use std::path::Path as FsPath;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use engine::RuntimeEngine;
use notify::Watcher;
use stdio as stdio_log;

static WATCHER_GUARDS: OnceLock<Mutex<Vec<notify::RecommendedWatcher>>> = OnceLock::new();

/// How long to keep draining the watch channel after the first event before
/// running one watch/evict/hmr cycle. A single editor save routinely delivers
/// several raw filesystem events for one logical change — write + `Close(Write)`
/// + a temp-file `Name(From)`/`Name(To)`/`Name(Both)` rename triad for
/// atomic-save editors — and without this window each one ran the full cycle
/// independently, tripling (or more) the work and the `[hmr]` output for one
/// save (deka#1067). Confirmed empirically: a single `sed -i` edit (glibc's
/// atomic-rename save pattern) produced 4 raw notify events, all within this
/// window.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(50);

pub(crate) fn start_watch(
    handler_path: &str,
    engine: Arc<RuntimeEngine>,
    dev_mode: bool,
    verbose: bool,
) -> Result<(), String> {
    let path = FsPath::new(handler_path);
    let project_root = project_root_from_handler(handler_path);
    let watch_root = project_root
        .as_deref()
        .unwrap_or_else(|| path.parent().unwrap_or_else(|| FsPath::new(".")));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<notify::Result<notify::Event>>();

    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .map_err(|err| err.to_string())?;

    watcher
        .watch(watch_root, notify::RecursiveMode::Recursive)
        .map_err(|err| err.to_string())?;

    // Keep watcher alive for process lifetime; dropping it stops event delivery.
    if let Ok(mut guards) = WATCHER_GUARDS.get_or_init(|| Mutex::new(Vec::new())).lock() {
        guards.push(watcher);
    }

    tokio::spawn(async move {
        loop {
            let Some(first) = rx.recv().await else {
                break;
            };

            // Drain whatever else lands within the debounce window so one
            // save collapses into one cycle (deka#1067) instead of one per
            // raw filesystem event.
            let mut batch = vec![first];
            while let Ok(Some(event)) = tokio::time::timeout(WATCH_DEBOUNCE, rx.recv()).await {
                batch.push(event);
            }

            let mut changed: Vec<String> = Vec::new();
            for event in batch {
                match event {
                    Ok(event) => {
                        for path in &event.paths {
                            if should_ignore_watch_path(path) {
                                continue;
                            }
                            let normalized = path.to_string_lossy().replace('\\', "/");
                            if !changed.iter().any(|existing| existing == &normalized) {
                                changed.push(normalized);
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!("watch error: {}", err);
                    }
                }
            }
            if changed.is_empty() {
                continue;
            }
            // Atomic-save editors (and `sed -i`) route the write through a
            // temp file, then rename it over the target; the temp path shows
            // up as its own changed path but is gone by the time the batch is
            // processed. Drop paths that no longer exist so the one user
            // save reports the one file the user edited — unless nothing in
            // the batch exists, which means a real deletion and must still
            // be reported (deka#1067).
            let survivors: Vec<String> = changed
                .iter()
                .filter(|path| FsPath::new(path.as_str()).exists())
                .cloned()
                .collect();
            if !survivors.is_empty() {
                changed = survivors;
            }

            if let Some(root) = project_root.as_ref() {
                if runtime_core::dist::is_source_app_router_project(root) {
                    if crate::build_watch::on_watch_event(root, &changed, dev_mode, verbose) {
                        let _ = engine.pool().evict_all().await;
                    }
                    if let Err(err) = runtime_core::dist::write_app_router_entry(root) {
                        tracing::warn!(
                            "failed to regenerate serve-entry after {}: {err}",
                            changed.join(", ")
                        );
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
            let evicted = engine.pool().evict_all().await;
            if evicted > 0 && verbose {
                stdio_log::log("watch", &format!("evicted {}", evicted));
            }
            if dev_mode {
                stdio_log::log("hmr", &format!("changed {}", changed.join(", ")));
                // Evict first so html-update re-renders the new source
                // rather than a stale isolate (#956).
                if !crate::refresh::push_js_update(&changed) {
                    transport::notify_hmr_changed(&changed);
                }
            }
        }
    });

    Ok(())
}

pub(crate) fn project_root_from_handler(handler_path: &str) -> Option<std::path::PathBuf> {
    let mut current = std::path::Path::new(handler_path).parent()?;
    loop {
        if current.join("deka.json").is_file() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

fn should_ignore_watch_path(path: &FsPath) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        return true;
    }

    if normalized.ends_with("/deka.lock") {
        return true;
    }

    // Generated/transient paths that should not trigger HMR loops.
    if normalized
        .split('/')
        .any(|seg| matches!(seg, ".cache" | "node_modules" | "target" | ".git"))
        || normalized.ends_with("/ds_modules")
        || normalized.ends_with("/php_modules")
    {
        return true;
    }

    false
}
