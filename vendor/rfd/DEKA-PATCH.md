# Deka rfd error-preserving patch

Source: crates.io rfd 0.17.2 (MIT license retained). Existing Option-returning
APIs remain unchanged. Deka uses added try_pick_file/try_save_file methods:
macOS distinguishes OK, Cancel and failure responses and checks the result URL;
Windows treats only ERROR_CANCELLED as cancellation and preserves HRESULTs;
Linux xdg-portal/zenity reports malformed responses and fallback backend errors.
Other optional backends fail explicitly for these added methods.

This avoids reporting native backend failures as user cancellation. Host service
errors reach both the Result callback and the configured application error sink.

The vendored macOS bindings also remove compiler-identified redundant unsafe
blocks and use CFUserNotification::display_alert, matching objc2 0.3.2 without
adding warnings. The Linux portal handles failed/disconnected responses and
Zenity distinguishes exit 1 (Cancel) from process failure.
