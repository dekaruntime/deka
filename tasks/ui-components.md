# APS 74 phase 5 — first component slice

The approved fallback slice is Button, Input, keyed List and Tabs. Badge, Toast,
modal focus-trapped Dialog (including Escape and focus restoration) and Menu
remain the next slice. This branch starts at origin/main; it does not depend on
or merge the unmerged hot-reload phase-2 PR #1447.

Components are ordinary typed `#[component]` functions returning `View`. They
use the existing signal runtime, native store and renderers. View::keyed owns
per-key registrations; the store grafts existing root handles in author order.
Row factories run once per mounted key, not on reorder. Effective writes and
component-local state remain attached to that allocation. Removed row owners
are disposed; re-addition creates a fresh allocation. Duplicate edits report an
error and preserve the last valid topology.

Tabs use automatic activation, horizontal wraparound arrows, Home/End, disabled
skipping and roving tab indices. Panel factories mount only the active contents;
persistent panel state belongs in parent signals. Focus requests are consumed
and validated by the originating host. The native host now delivers keys to
non-editor controls. Browser DOM traversal follows keyed author order and uses
mount-scoped IDs for control relationships. Roles, labels and selected state
share the effective retained semantics used by AccessKit and the browser host.

Theme tokens copy zega.dev's current light/dark palette (retrieved 9 October
2026): warm surfaces with a green action accent. Themes are explicit signals.
Enter/Exit presets use existing renderer presence; Button activation uses a
finite Press timeline. Browser reduced-motion preferences and the native
host's explicit reduced-motion policy snap all three. Native editor glyphs,
caret/composition decorations and browser text overlays now use the effective
foreground so dark-theme input values stay readable.

Showcase sources, themed constructors and expectations share the compiled /tour
package pipeline. The original 27 DekaScript parity lessons retain their own
unchanged reference gate. No per-request server computation or new dependencies
are introduced. Eight production offscreen PNGs are included with the component
documentation; the screenshot gate captures the complete native editor paint,
not only bare retained nodes.

Validation tools:

- `./scripts/test-rust-ui.sh` — shared interactions, lifecycle, motion, macro
  diagnostics and all 27 existing tour parity lessons.
- `cargo test --locked --release -p deka-ui --features tour,web --test components_native`
  — native pointer/key/focus/selection/clipboard, AccessKit and eight GPU shots.
- `python3 scripts/rust-ui-components/screenshots.py` — encode RGBA shots as PNG.
- `python3 scripts/rust-ui-components/revert.py` — temporarily remove each
  behavioural path, require the named interaction test to fail, restore every
  source file in finally, then require the restored suite to pass.
- `python3 scripts/build-rust-tour.py --out .tmp/components-tour` and
  `node scripts/rust-tour-browser/check.mjs .tmp/components-tour` — real headless
  Chromium, shared renderer histories, DOM editing/traversal and tab focus.
- `python3 scripts/rust-ui-components/browser-revert.py .tmp/components-tour`
  — remove the packaged draw guard, require the original image-loss assertion
  to fail, restore the package in finally, then pass the complete browser gate.

Acceptance results and size measurements belong in the PR body. Build logs and
raw screenshot evidence remain under this checkout's `.tmp`. Run one Cargo
build at a time, with CARGO_TARGET_DIR set to this checkout's `.target` and TMPDIR
set to `.tmp`. No visible windows, runner-directory access, push, merge, tagging,
release or CI polling is part of this task. Ava receives the signed branch via
`.tmp/components.bundle` and the review body via `.tmp/components-pr-body.txt`.

-codex
