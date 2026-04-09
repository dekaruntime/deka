# MVP2 JS Runtime Plan

Status: Completed
Owner: Runtime

## Mission
Move PHPX execution to a JS-first runtime: transpile PHPX to JS and execute via ESM with Rust ops providing host capabilities.

## Goals
- PHPX -> JS is the default execution model.
- ESM module loading is the runtime source of truth.
- Rust ops provide the host boundary for non-standard behavior.
- Dev: on-the-fly transpile + cache. Prod: build output only.

## Non-goals
- Keep PHPX/wasm execution in the hot path.
- Reintroduce Node/Bun compatibility layers.
- Add a parallel JS stdlib tree (we transpile PHPX stdlib instead).

## Workstreams
1. **Runtime prelude for JS execution**
   - [x] Provide `bridge(...)` + core helpers expected by transpiled stdlib.
   - [x] Map ops to existing Rust host behavior.

2. **JS module loading**
   - [x] ESM loader in runtime consumes transpiled JS.
   - [x] Cache transpiled output in project `.cache/phpx_js`.

3. **Transpilation pipeline**
   - [x] Use PHPX -> JS emit for all modules (stdlib + user code).
   - [x] Ensure module graph integrity and hashing remains stable.

4. **Dev workflow**
   - [x] Hot transpile on file change (isolate cache evict + ESM reload).
   - [x] Clear errors if transpile fails.

5. **Build workflow**
   - [x] `deka build` emits JS bundles for server/client.
   - [x] Single output file for server/client with SWC bundler.

6. **Compatibility + tests**
   - [x] Conformance tests updated to run via JS runtime.
   - [x] E2E tests for router + components + stdlib.

## Acceptance Criteria
- `deka run app/main.phpx` executes through JS runtime path by default.
- `deka serve app/main.phpx` serves via JS runtime path.
- Stdlib modules execute via transpiled JS + ops.
- Bundle outputs run under Node without missing stdlib symbols (where supported).

## Verification
- `cargo build --release -p cli`
- PHPX conformance suite: `PHPX_BIN=target/release/cli PHPX_BIN_ARGS=run bun tests/phpx/testrunner.js`
- At least one JS runtime E2E (router + stdlib JSON).
- `scripts/e2e-js-runtime-router-components-stdlib.sh`
- `scripts/e2e-mvp2-run-serve-build.sh`
- `scripts/e2e-js-runtime-fallback-diagnostics.sh`

## Progress Notes
- 2026-02-22: PHPX conformance green in `mvp2` (`59/59` via `node tests/phpx/testrunner.js`).
- 2026-02-22: `deka init` now writes `php_modules/stdlib.json` from discovered module graph so fresh projects can run `deka build` without missing stdlib manifest errors.
- 2026-02-22: Added smoke coverage scripts for run/serve/build and fallback diagnostics.
