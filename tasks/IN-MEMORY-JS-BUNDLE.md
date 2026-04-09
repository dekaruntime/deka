# In-Memory PHPX -> JS Bundle (MVP)

Status: Completed
Owner: Runtime/CLI

## Goal
Provide a single-command build path that emits JS from PHPX and bundles it in-memory (no intermediate JS files on disk), producing a single output bundle for client or server.

## Why
- Avoid transient JS artifacts on disk.
- Enable a clean `deka build --bundle` flow.
- Set up the JS target for eventual runtime use without requiring a separate bundling step.

## Scope
- Add a new CLI entry (or flag) that performs: parse/typecheck -> JS emit (in-memory) -> SWC bundle -> write final bundle.
- Use a virtual module for the in-memory JS entry.
- Resolve module specifiers (`@/`, `component/`, `encoding/`, `deka/`, `db/`, relative imports) against the project root / `php_modules`.
- Emit a single JS output file (optionally minified).
- Ensure no intermediate JS files are written.

## Non-Goals
- Re-introduce full JS/TS runtime execution in `deka run/serve`.
- Replace the PHPX runtime path yet.
- Rebuild the removed JS runtime pipeline.

## Tasks
1. **CLI surface** ✅
   - Add `deka build --bundle` (or `deka bundle`) path.
   - Decide output naming (default `dist/client/bundle.js` / `dist/server/bundle.js`).

2. **In-memory JS emit** ✅
   - Expose JS emitter output as a string from `cli build` pipeline.
   - Avoid writing the intermediate JS file when `--bundle` is set.

3. **SWC bundler integration** ✅
   - Add a virtual entry module (e.g. `virtual:phpx-entry`).
   - Implement `Resolve` + `Load` that map:
     - `@/` -> project root
     - `component/` -> `php_modules/component/`
     - `encoding/` -> `php_modules/encoding/`
     - `deka/` -> `php_modules/deka/`
     - `db/` -> `php_modules/db/`
     - relative imports -> relative to the importing file
   - Return the in-memory JS for the entry module path.

4. **Tree-shake + minify (optional)** ✅
   - Hook SWC minifier with default conservative options.
   - Make minify opt-in via `--minify`.

5. **Tests** ✅
   - Add a fixture that imports local modules and assert bundle output inlines imports.
   - Validate that no intermediate JS file is created.

6. **Docs** ✅
   - Update `docs/phpx/phpx-js-target-semantics.md` with bundle path and behavior.
   - Add a short DX note to `docs/phpx/phpx-dx.md`.

## Verification
- `cargo build --release -p cli`
- `deka build --bundle app/main.phpx --out /tmp/deka-bundle.js`
- `node /tmp/deka-bundle.js` (for a no-stdlib fixture)
- Confirm no intermediate JS emit file on disk.
