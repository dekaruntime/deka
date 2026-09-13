# Phase 2 validation

- `node bench/run.mjs` completed successfully on 2026-09-13; `phase2.json` identifies the exact source commit and toolchain hashes.
- Accepted sample counts, medians, CDP trace endpoints, and payload sums independently checked against the JSON. Static Deka JS is zero; static Next JS is nonzero. Both stories use the same post URL.
- Production Chromium theme checks verify button state, document theme, and computed background color; newsletter checks verify submitted state on all three stacks.
- Existing release tests, with `CARGO_TARGET_DIR` isolated to this clone and `DEKA_DSC=bench/.toolchain/dsc`:
  - `cargo test --release -p runtime_core dist::codegen`: 13 passed.
  - `cargo test --release -p deka_http --features dev-server --lib`: 52 passed.
  - `cargo test --release -p cli --features dev-server --test fast_refresh`: 2 passed.
- Separate CDP client-boundary probe: editing a mounted Label preserves its sibling Counter at 1 and does not reload. The benchmark's document-only PostCard correctly reloads instead.
- Workspace test files are unchanged. Linux was not run on this macOS host; platform guards remain and the contention guard uses macOS/Linux `ps`.
- `git diff --check` passed; commits use Sami's verified SSH signing identity.
- Runtime docs updated in `docs/dekascript/fast-refresh.mdx`. The repository-guideline publish command was attempted but cannot run: `scripts/publish-docs.js` does not exist in this checkout (`MODULE_NOT_FOUND`). No website publication is claimed.

The contention guard records the shared host's competing compiler activity. It checks boundaries and every 500 ms; it is not a complete record of all desktop activity. Rejected values remain in the JSON and are not selected by their duration. No other lane was paused.
