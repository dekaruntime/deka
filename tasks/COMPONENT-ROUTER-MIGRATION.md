# Component Router Migration (Userland App Routing)

Goal: Move app routing + page resolution out of runtime magic and into userland `php_modules/component/router.phpx`, while keeping a clean, NextJS-like DX.

## Task 1: Inventory + Baseline
- [x] Document current routing behavior and entry resolution.
- [x] Capture current behavior and error cases in notes.
- [x] Add a regression fixture for `deka init` + `deka task dev` 404 case.

Notes (current behavior):
- Runtime resolves handler path in Rust: `crates/core/src/handler.rs` + `crates/engine/src/config.rs`.
- If an `app/` directory exists, runtime forces PHP routing mode and treats the project root as the handler path.
- PHP routing is implemented in `crates/modules_php/src/modules/deka_php/php.js` (`buildAppRouteManifest`, `resolvePhpDirectoryRoute`, `resolvePhpApiRoute`).
- `deka init` writes `app/main.phpx`, but the current router expects `app/page.phpx` / `app/index.phpx`, so a fresh init can 404 with “Not Found”.

## Task 2: Userland Router API (PHPX)
- [x] Define `component/router.phpx` public API:
  - `resolve($request, $root, $opts)`
  - `Router($props)` component
  - `route_manifest($root, $opts)`
- [~] Port routing logic from `deka_php/php.js` into PHPX.
- [ ] Ensure compatibility with existing `app/` conventions (`page.phpx`, `layout.phpx`, dynamic tokens).

Notes:
- Added `component/router.phpx` with `generate_manifest()` scaffold.
- Added `fs.readDirSync` + `fs.mkdirSync` via fs bridge so router can scan `app/` and write `php_modules/.cache/app-manifest.json`.
- Added `load_manifest()` + `resolve_path()` + token matching helpers (parity with runtime router).
- Manifest now includes `module_id` for `.phpx` routes and caches in memory per process.
- Router now renders `Page` + `Layout` exports and returns partial responses when requested.

## Task 3: Userland Serve Entry
- [x] Update `deka init` template to generate `app/main.phpx` that calls the router.
- [x] Ensure `renderToString` is default (alias `render()`), and `renderToStream` optional.
- [ ] Provide explicit error messages when `app/` is missing or empty.

Notes:
- `deka init` now scaffolds `app/page.phpx` and `app/layout.phpx` plus `component/router.phpx`.
- `app/main.phpx` now calls `Router()` by default.
- Runtime prelude now preloads app module IDs once per process (respects security gates).
- Runtime now treats `.php` as unsupported for routing and run mode.

## Task 4: Runtime De‑Magic (Phase 1)
- [ ] Gate the old runtime router with a warning.
- [ ] Prefer explicit `serve.entry` and userland router.
- [ ] Add a migration warning when runtime app routing is used.

## Task 5: Tests + Docs
- [ ] Add PHPX fixtures for router behavior (static, dynamic, layout).
- [ ] Update docs: “App Routing” and “Router Component” usage.
- [ ] Run PHPX conformance test suite.
