# Phase 2 validation

- `node bench/run.mjs` completed successfully on 2026-09-13; `phase2.json` identifies the exact source commit and toolchain hashes.
- Accepted sample counts, medians, CDP trace endpoints, and payload sums independently checked against the JSON. Static Deka JS is zero; static Next JS is nonzero. Both stories use the same post URL.
- Production Chromium theme checks verify button state, document theme, and computed background color; newsletter checks verify submitted state on all three stacks.
- Existing release tests, with `CARGO_TARGET_DIR` isolated to this clone and `DEKA_DSC=bench/.toolchain/dsc`:
  - `cargo test --release -p runtime_core dist::codegen`: 13 passed.
  - `cargo test --release -p deka_http --features dev-server --lib`: 52 passed.
  - `cargo test --release -p cli --features dev-server --test fast_refresh`: 2 passed.
- Separate CDP client-boundary probe: editing a mounted Label preserves its sibling Counter at 1 and does not reload. The benchmark's document-only PostCard correctly reloads instead.
- The review fix adds committed real-process HTTP/WS regression coverage and a serve-response golden. Linux was not run on this macOS host; platform guards remain and the contention guard uses macOS/Linux `ps`.
- `git diff --check` passed; commits use Sami's verified SSH signing identity.
- Runtime docs updated in `docs/dekascript/fast-refresh.mdx`. The repository-guideline publish command was attempted but cannot run: `scripts/publish-docs.js` does not exist in this checkout (`MODULE_NOT_FOUND`). No website publication is claimed.

**PRELIMINARY — pending one clean idle-host rerun.** Numbers were measured on a shared host with the contention guard (**10 waits / 8 discards**). The guard mitigates compiler contention; it does not make this an isolated run. Ava schedules the clean idle-host rerun before homepage use. No benchmark rerun was performed for this review fix.

The contention guard records the shared host's competing compiler activity. It checks boundaries and every 500 ms; it is not a complete record of all desktop activity. Rejected values remain in the JSON and are not selected by their duration. No other lane was paused.

## PR #955 review regression coverage

- Real `deka dev` app-router requests assert HTML and fragment Content-Type, HTML HMR-client injection, and no HMR-client injection into the JSON fragment.
- Real filesystem edits and WebSocket frames exercise both paths: document `js-update` with the exact family ID and plain-handler `reload`. The very first HTTP request after either notification must contain the saved source; there is no stale-response retry.
- The document WS payload is passed to the shipped `refresh.js` with the real vendored React Refresh runtime. It must reload synchronously for an unregistered family. Separate Node coverage checks mixed known/missing families, no imports before fallback, and a successful registered-family update. These are client behavior tests with supplied browser globals, not a browser DOM/state-preservation claim.
- The generated `respond` function matches a committed golden, including HTML, JSON, and serialization-error Content-Type headers.
- Retained all four original production changes: a real document-component edit emits `js-update` even without hooks, so removing the family fallback would be incorrect. Removing that guard in a temporary test copy made the missing-family regression assertion fail.
- Focused release checks: CLI Fast Refresh 4 passed; HTTP React Refresh 15 passed; serve-response golden 1 passed.
- Requested gate passed: `NO_COLOR=1 FORCE_COLOR=0 cargo test --locked --workspace --features cli/dev-server` (including doc tests). The target directory was isolated to this clone; `DEKA_DSC` selected `bench/.toolchain/dsc` (0.52.2), also installed at `target/release/dsc` for runtime compiler discovery. Test/dev debug symbols were disabled and Cargo jobs capped at 4 to bound local disk/build load.
