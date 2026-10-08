# Draft: Add fallible synchronous single-file open/save dialogs

Target: `rust-windowing/rfd`. Ava files the external PR; this document does not
represent an already submitted PR. Based on crates.io rfd 0.17.2, upstream commit
`cca0cfa5fc38c2752c9c98f9f5168904a515fd55`. Reconcile with upstream main before filing.

## Description

When a native file panel fails, `pick_file()` and `save_file()` return `None`, the
same value as user cancellation. Applications cannot show a backend failure or
retry it without misreporting a user's Cancel action.

Add `FileDialog::try_pick_file()` and `FileDialog::try_save_file()`, returning
`Result<Option<PathBuf>, String>`: `Ok(Some(path))` for a selection, `Ok(None)`
for cancellation, and `Err(message)` for a backend/result failure. Existing public
signatures remain available. No new dependency is required.

- macOS: only `NSModalResponseCancel` is cancellation. Other non-OK modal
  responses retain their code. Accepted panels with no result URL or URL path
  return an error rather than panicking.
- Windows: preserve COM initialization, panel construction, Show and result
  HRESULTs. Only `HRESULT_FROM_WIN32(ERROR_CANCELLED)` maps to cancellation.
- Linux XDG portal: distinguish Cancel from failed portal responses, detect
  disconnects, and reject invalid file URIs. Retain Zenity fallback when the portal
  fails; a successful fallback may still return a selection or cancellation.
  Zenity status 1 is cancellation, while other failed statuses preserve stderr.
- Backend trait defaults report unsupported fallible dialogs explicitly. GTK3
  implementation, async methods, multi-file and folder APIs are outside this patch.

The portal and Zenity helper corrections are shared with existing APIs; those
signatures remain compatible, but failed portal responses now trigger fallback and
Zenity failures are no longer accepted merely because stdout is nonempty.

## Patch scope

The exact source delta is enumerated in [DEKA-PATCH.md](DEKA-PATCH.md). Include
only the listed `src/` changes when preparing the upstream branch, with these
macOS compatibility fixes split into a prerequisite commit if preferred:
redundant unsafe removal, needless borrow removal, checked-then-unwrap cleanup,
and `CFUserNotification::display_alert` for the resolved objc2 0.3.2 bindings.
Do not send Deka's path dependencies, workspace lockfiles or vendoring documents
upstream. Upstream maintainers may prefer a structured error type; `String` is
the current additive implementation and retains the native diagnostic codes.

## Validation

On macOS, with Deka's vendored source:

```sh
cargo test --locked --release --manifest-path vendor/rfd/Cargo.toml --lib
cargo clippy --locked --release --manifest-path vendor/rfd/Cargo.toml --lib -- -D warnings
```

The modal-result regression test covers success, cancellation without reading the
result, abort/non-OK response, and missing URL. Removing failure preservation makes
that regression fail. Deka's consumer tests separately cover selected paths,
cancellation and error delivery to callback and application sink. No interactive
panel is opened by these checks. Windows/Linux execution and upstream CI are
still needed before acceptance; no result for those platforms is claimed here.

## Release follow-up

[dekaruntime/deka#1439](https://github.com/dekaruntime/deka/issues/1439) tracks replacing its vendored source with the first upstream release that
preserves these outcomes. The consumer will switch its native renderer's dependency to the released crate,
update the root lockfile, verify the standalone packager's locked dependencies,
and rerun the native-service and relocated-package checks.

-codex
