use std::path::Path as FsPath;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use engine::RuntimeEngine;
use notify::Watcher;
use stdio as stdio_log;

static WATCHER_GUARDS: OnceLock<Mutex<Vec<notify::RecommendedWatcher>>> = OnceLock::new();

pub(super) fn start_watch(
    handler_path: &str,
    engine: Arc<RuntimeEngine>,
    dev_mode: bool,
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
        while let Some(event) = rx.recv().await {
            match event {
                Ok(event) => {
                    let mut changed: Vec<String> = Vec::new();
                    for path in &event.paths {
                        if should_ignore_watch_path(path) {
                            continue;
                        }
                        let normalized = path.to_string_lossy().replace('\\', "/");
                        if !changed.iter().any(|existing| existing == &normalized) {
                            changed.push(normalized);
                        }
                    }
                    if changed.is_empty() {
                        continue;
                    }

                    if let Some(root) = project_root.as_ref() {
                        if runtime_core::dist::is_source_app_router_project(root) {
                            if crate::build_watch::on_watch_event(root, &changed, dev_mode) {
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
                    if dev_mode {
                        stdio_log::log("hmr", &format!("changed {}", changed.join(", ")));
                        transport::notify_hmr_changed(&changed);
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    let evicted = engine.pool().evict_all().await;
                    if evicted > 0 {
                        stdio_log::log("watch", &format!("evicted {}", evicted));
                    }
                }
                Err(err) => {
                    tracing::warn!("watch error: {}", err);
                }
            }
        }
    });

    Ok(())
}

fn project_root_from_handler(handler_path: &str) -> Option<std::path::PathBuf> {
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
