# Native menus and file dialogs (APS 74, phase 4a)

Create a `DesktopApp` and build menu callbacks inside its app closure or a
window factory/handler. `windows.app_menu(Menu::new().standard_app("deka")
.submenu("Actions", Menu::new().item(MenuItem::new("Add", handler))))` queues
an application menu. On macOS it is the application menu bar; on Windows the
same menu attaches to each window. `MenuItem::accelerator("CmdOrCtrl+I")`
parses a shortcut and returns an error for invalid syntax. Disabled items are
disabled in the native menu and rejected by the event dispatcher.

`onContextMenu={move |event| ...}` receives `Event::ContextMenu { x, y }` in
logical window coordinates. Right-clicks bubble from the hit retained node to
its nearest registered ancestor. Call `window.context_menu(menu, x, y)` from
the handler. The Context Menu key and Shift-F10 use the focused control.
Callbacks retain their originating scope and tree weakly; closed windows lose
their popup routes. Menu actions arrive as muda events through the winit proxy,
then update signals in a batch. A callback can open/close windows through the
same queued window capabilities.

Menus use [muda 0.21.1](https://github.com/tauri-apps/muda), a maintained
Apache-2.0/MIT native AppKit/Win32 library, without its default GTK feature.
Windows shortcuts use winit's message hook with `TranslateAcceleratorW`.
Linux native menu attachment is not implemented: muda needs a GTK host, while
deka currently owns winit windows without GTK. Adding a second host and its
system dependencies needs a platform decision; this limitation is recorded on
deka#1434. The menu API is therefore only exported on macOS/Windows, rather than
silently ignoring requests on Linux.

Use `window.open_file(FileDialogOptions::new().filter("Text", &["txt", "md"]),
completion)` or `window.save_file(options, completion)`. Options include title,
directory, initial filename and extension filters. Completion receives
`Result<Option<PathBuf>, String>`: a selected path, cancellation, or a host
error. Selection does not read/write the file. Requests run after the handler's
event batch; the OS panel is modal and keeps its owning window alive until it
returns. Completion re-enters the originating scope/tree, with window-local
allocations disposed on close.

Dialogs use [rfd 0.17.2](https://github.com/PolyMeilex/rfd), a maintained MIT
native dialog library. Its default Linux backend is the desktop portal;
macOS and Windows use their system panels. No webview is introduced. Browser
code has no native OS menu/window/file-dialog capability. Browser context events
can update the Rust tree; the browser's own context menu remains available when
Rust has no handler. Browser file selection/download APIs need a separate
capability and result model and are outside this native API.

Run `cargo run --release -p deka-ui --example native_services` for the visible
demo. `-- --snapshot <file.ppm>` captures the handler's rendered result headlessly
with an injected dialog provider. On macOS, `-- --menu-smoke` constructs actual
NSMenu objects without displaying a window, invokes Cocoa's target/action,
consumes the resulting muda event and asserts the rendered signal changed.
`-- --dialog-smoke <existing-directory>` explicitly opens a small winit owner
window and genuine file panels. It posts synthetic AppKit mouse events into the
real winit path, opens filtered Open/Save panels from Rust handlers, cancels both,
asserts completion updates and closes the owner automatically. It never writes
a selected file. Capture attempts address only the owned window/panel IDs.
On the current host those attempts fail with "could not create image from
window"; native panel/menu visuals and actual file choices remain manual checks.
The headless `--snapshot` mode supplies the rendered Rust form image instead.

Manual checks requiring visible app windows: inspect the menu bar, shortcut,
right-click popup and dismissal; choose an Open file and Save destination;
confirm only the selected path updates and cancellation preserves it. These
steps also cover native VoiceOver menu navigation. Automated tests enter via
winit cursor/mouse events and public native menu event payloads, asserting
rendered shared state, originating-tree edits, closed-window rejection, filters,
selection, cancellation and backend errors. The headless driver shares the
production service request consumer and callback routing.
