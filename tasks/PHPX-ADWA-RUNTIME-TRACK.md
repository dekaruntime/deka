# PHPX Adwa Runtime Track

Status: active
Owner: runtime + adwa
Scope: run PHPX inside browser via `adwa` host profile

## Goals

- Treat `adwa` as the primary browser host target for PHPX.
- Keep a single PHPX language model across server and browser hosts.
- Gate capabilities by host contract instead of ad-hoc browser checks.
- Enable playground/repl workflows with realistic module/runtime behavior.

## Host Alignment (locked)

- [x] Browser target is `adwa`-first (not generic browser stubs).
- [x] Capability-driven runtime behavior by host profile.
- [x] Same module/type semantics across `server` and `adwa`.
- [x] Internal bridge remains private; host adapters implement capabilities.

## Host Profiles

- [x] Define explicit host profile enum/config: `server`, `adwa`.
- [x] Add capability matrix consumed by runtime + validation:
  - [x] `fs`
  - [x] `net`
  - [x] `process/env`
  - [x] `clock/random`
  - [x] `db`
  - [x] `wasm imports`
  - Note: runtime gating, compiler validation, and LSP target-aware diagnostics are all implemented for restricted imports.
- [x] Ensure unsupported operations return structured capability errors.

## Runtime Integration

- [x] Build/pack php-rs runtime wasm artifact for `adwa` integration tests.
- [x] Add host adapter layer mapping PHPX runtime ops to `adwa` interfaces.
- [x] Verify fs path maps to `adwa` in-memory/mounted filesystem.
- [x] Verify process I/O maps to `adwa` stdio channels.
- [x] Verify module loading from virtual FS.

## Module System + Imports

- [x] Support `@/` alias against virtual project root in `adwa`.
- [x] Resolve `php_modules/` via mounted/virtual tree, not disk-only assumptions.
- [x] Ensure generated cache/lock behaviors work in virtual FS mode.
- [x] Add tests for module graph load under `adwa`.

## Validation + LSP target awareness

- [x] Add target-mode setting for diagnostics: `phpx.target = server|adwa`.
- [x] Emit capability-aware errors (forbidden modules/APIs in target host).
- [x] Keep suggestions actionable (e.g. alternate API or host config).
- [x] Add LSP tests for target-specific diagnostics.

## DX + Tooling

- [x] Add developer script for `adwa` php runtime smoke run.
- [x] Add playground boot script/example using `adwa` with PHPX file tree.
- [x] Ensure docs explain host capabilities and limitations clearly.

## Testing

- [x] Runtime smoke tests in `adwa`:
  - [x] basic PHPX eval
  - [x] imports/module graph
  - [x] JSX render-to-string
  - [x] filesystem read/write
- [x] Negative tests: capability denied errors for blocked APIs.
- [x] Browser e2e: edit -> run -> output updates in playground (Node shim path).
- [x] Browser e2e for PHPX path via runtime adapter (`ADWA_E2E_INCLUDE_PHPX=1`).

## Boundary Enforcement

- [x] Added architecture guard against direct `php-rs` imports in browser demo code.
- [x] Browser e2e defaults to stable path only; PHPX browser spec is opt-in.
- [x] Runtime adapter path for PHPX browser execution.
- [x] Close `tasks/PHPX-ADWA-RUNTIME-BOUNDARY.md`.

## Docs

- [x] Add `docs/phpx/general/adwa-browser-runtime.mdx`.
- [x] Document host model, capability matrix, and how to run demos.
- [x] Include migration notes from server-only assumptions.

## Cross-track dependency

- [ ] Async track (`tasks/PHPX-ASYNC-TRACK.md`) feeds directly into `adwa` host event scheduling and top-level await behavior.

## Exit Criteria

- [ ] PHPX runs in browser via `adwa` with module imports and JSX SSR output.
  - Note: base PHPX execution is now live in browser via adapter + wasm executor.
- [x] Capability errors are explicit and stable.
- [x] LSP can validate against `adwa` target mode.
