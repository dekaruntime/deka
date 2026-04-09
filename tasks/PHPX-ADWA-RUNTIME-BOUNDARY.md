# PHPX Adwa Runtime Boundary

Status: complete  
Owner: runtime + adwa

## Purpose

Define and enforce the browser runtime boundary for PHPX in ADWA so demos and tooling do not bypass runtime architecture.

## Non-negotiable Rules

- [x] Browser demo code must not import `php-rs` wasm artifacts directly.
- [x] Browser demo code must not call raw wasm exports (`php_alloc/php_run/php_free`) directly.
- [x] Browser execution flows through a runtime adapter API owned by Deka/ADWA.
- [x] Browser execution scaffold now flows through a runtime adapter API owned by Deka/ADWA (`adwa/js/src/phpx_runtime_adapter.ts`).
- [x] Adapter exposes one stable execution contract for PHPX source runs.
- [x] Internal bridge remains private and undocumented for userland.

## Execution Contract (v1)

- [x] Define `run(source, mode, context) -> result` shape:
  - [x] `ok: bool`
  - [x] `stdout: string`
  - [x] `stderr: string`
  - [x] `diagnostics: []`
  - [x] `meta: object`
- [ ] Contract errors must be structured (no opaque string-only failures).
- [x] Contract errors are structured (diagnostics + meta, no opaque string-only failures).
- [x] Contract must support host capability reporting.

## Capability Surface

- [x] Explicit host capability map for browser profile:
  - `fs`
  - `net/fetch`
  - `env`
  - `clock/random`
  - `db`
  - `wasm imports`
- [x] Capability-denied responses must map to actionable diagnostics.

## Guardrails

- [x] E2E script fails if browser demo imports `vendor/php_rs` directly.
- [x] Browser e2e defaults to stable Node-shim spec only.
- [x] PHPX browser spec remains available behind explicit opt-in (`ADWA_E2E_INCLUDE_PHPX=1`).
- [x] Added second static guard for direct raw `WebAssembly.instantiate` in browser demo entry.

## Test Plan

- [x] Unit/smoke test runtime adapter response shape.
- [x] Smoke test runtime adapter response shape (`npm run smoke:phpx-runtime-adapter`).
- [x] Integration test adapter + capability-denied handling.
- [x] Browser e2e for PHPX path once adapter wiring lands.
- [x] Keep Node-shim e2e green throughout.

## Exit Criteria

- [x] PHPX browser demo uses runtime adapter only.
- [x] No direct php-rs wiring in demo layer.
- [x] Stable e2e coverage for both Node shim and PHPX adapter modes.
