# Multiple windows (APS 74, phase 4a)

`launch(DesktopApp::new(|windows| { ... }))` runs one winit event loop.
Call `windows.open(WindowOptions::new("Title", 640., 480.), |window| view! { ... })`
for each initial window. Clone the manager into a Rust event handler to open
another window; call `window.close()` to queue its close. Requests run after the
current event batch. Closing the last window exits the app.

Allocate shared signals in the `DesktopApp` construction closure, before the
window factories. Each factory mounts an independent retained tree on that
scope. Signals/effects allocated inside a factory or its event handlers belong
to that mount and are disposed on close. A reaction runs with its originating
tree context, even when another window changed its dependencies. Node references
and tree handles remain window-specific; sharing a scope does not permit moving
a node between windows.

Each window owns its surface, text editors, focus, AccessKit tree, scheduler and
options. A single app-wide text clipboard owner outlives any window that copied
text. Winit window IDs select the target; stale events for closed windows are
ignored. Shared signal updates wake affected window renderers.

The OS-window API is native-only. Browsers have different rules for creating
windows, so this API does not call `window.open`. The applicable shared-state
feature works through `UiApp::new_in_scope(&scope, factory)` and `web::mount_app`
on independently owned canvases. Each mount has a scheduled reactive wake;
dropping one web handle leaves other mounts and shared state alive.

Run `cargo run --release -p deka-ui --example multiple_windows` for the native
example. For a headless renderer capture, append `-- --snapshot <directory>`.
The headless multi-window driver consumes the same queued commands and winit
routing as the native event loop. Tests inject pointer/key/close events and
assert rendered state, cross-window updates, disposal and clipboard effects.
