# Deka patch to rfd 0.17.2

Base: the MIT-licensed crates.io `rfd` 0.17.2 source, upstream
[`cca0cfa5fc38c2752c9c98f9f5168904a515fd55`](https://github.com/rust-windowing/rfd/tree/cca0cfa5fc38c2752c9c98f9f5168904a515fd55).
`LICENSE`, manifests, build script and dependencies are unchanged. The standalone
`Cargo.lock` is regenerated for the resolved macOS dependencies. Deka's
native renderer selects this directory through its optional rfd path dependency.
The standalone packager does not currently enable that GPU dependency.

## Why this copy exists

The released synchronous `pick_file` / `save_file` APIs return `Option<PathBuf>`.
That collapses backend errors into cancellation. Deka must deliver cancellation
as `Ok(None)` and failures to both its `Result` completion callback and its
ordinary application error sink. The additive methods below provide that contract
without changing existing method signatures. Deka uses only these two methods.

```diff
 impl FileDialog {
+    pub fn try_pick_file(self) -> Result<Option<PathBuf>, String>;
+    pub fn try_save_file(self) -> Result<Option<PathBuf>, String>;
 }
```

This is an API summary; the methods delegate to the backend traits. No async,
multiple-file, folder or message-dialog error API is added.

## Source delta

| Files under `src/` | Change and reason |
| --- | --- |
| `file_dialog.rs`, `backend.rs` | Add two public methods and backend trait methods. Default implementations explicitly report unsupported backends; GTK3 cannot silently report cancellation. |
| `backend/macos/file_dialog.rs` | Run the existing panels; map OK to a checked path, Cancel to `Ok(None)`, and other modal responses to `Err` with the response code. Test success, cancel, abort and missing result URL. |
| `backend/macos/file_dialog/panel_ffi.rs` | Add `try_get_result` so a missing URL/path becomes an error instead of an unwrap panic. Remove redundant unsafe blocks and needless reference borrows reported with objc2 0.3.2. |
| `backend/win_cid/file_dialog.rs` | Preserve COM setup, panel construction, Show and result HRESULTs; only `0x800704c7` (`ERROR_CANCELLED`) maps to `Ok(None)`. |
| `backend/xdg_desktop_portal.rs` | Add checked single-file open/save routes; reject malformed file URIs and propagate fallback Zenity errors. Portal unavailability/failure still uses the existing Zenity fallback; successful fallback is accepted. |
| `backend/xdg_desktop_portal/portal/{mod.rs,libdbus.rs}` | Distinguish cancellation code 1 from failed responses and detect a disconnected D-Bus connection instead of waiting forever. These shared helpers also affect the legacy APIs. |
| `backend/linux/zenity.rs` | Treat status 0 as success, 1 as cancel, and other statuses as failures with stderr. This shared helper also affects the legacy APIs; their signatures stay unchanged. |
| `backend/macos/{message_dialog.rs,modal_future.rs,utils.rs,utils/policy_manager.rs,utils/user_alert.rs}` | macOS compatibility/warning cleanup: remove redundant unsafe, replace checked-then-unwrap with `if let`, and use `CFUserNotification::display_alert` from the resolved objc2 bindings. No new message-dialog API. |

Keep the delta limited to those source files, the standalone lockfile, and these
handoff documents. To inspect the exact source-only diff against the released
crate (using an unpacked crates.io source directory):

```sh
diff -ru rfd-0.17.2/src vendor/rfd/src
```

## Verification and removal

On macOS:

```sh
cargo test --locked --release --manifest-path vendor/rfd/Cargo.toml --lib
cargo clippy --locked --release --manifest-path vendor/rfd/Cargo.toml --lib -- -D warnings
```

The modal-result test does not display a panel. Deka's native-service tests also
exercise result/error/cancellation delivery with an injected dialog backend.
Windows and Linux source is included but has not been executed on this macOS host;
upstream review and platform validation remain required.

[UPSTREAM-PR.md](UPSTREAM-PR.md) is the draft description for Ava to file at
rust-windowing/rfd. It is not an opened external PR. Replace this path dependency
with a released upstream version once it provides the required error contract
([deka#1439](https://github.com/dekaruntime/deka/issues/1439));
remove the vendored tree, update the root lockfile and verify the standalone
packager's locked build/relocated bundle in that follow-up.

-codex
