# Security Prompt Reduction (Phase 1)

## Goal
Reduce prompt spam in `deka run/serve/task` while keeping default-deny behavior for userland code.

## Scope
- Focus on startup/runtime internal operations first (read/write/env/wasm noise).
- Do not weaken third-party/user module protections.

## Task List
- [x] Stabilize `deka init` so a fresh project boots reliably with current security defaults.
- [x] Tighten `deka init` security defaults to remove broad allow warnings (`read=.` / `env=*` / `wasm=*`).
- [x] Classify permission checks by origin:
  - [x] `runtime-internal` (module loading, cache materialization, manifest/lockfile reads).
  - [x] `project-owned` (app code + local modules).
  - [x] `third-party` (installed packages, registry artifacts).
- [x] Enforce privileged bypass only for `runtime-internal` operations.
- [x] Collapse repeated prompts into scoped decisions:
  - [x] directory-level read/write grants for current process session.
  - [x] capability-level env grants for known safe runtime keys.
- [x] Improve denial errors with structured suggestions:
  - [x] exact `deka.json` path/key to edit.
  - [x] minimal allow rule to unblock.
  - [x] short risk note.
- [x] Add regression tests:
  - [x] fresh init + `deka task dev` boots without interactive prompt cascade.
  - [x] userland package access still prompts/denies correctly when not allowed.
  - [x] third-party module remains denied by default without explicit rule.

## Exit Criteria
- `deka init` project runs with a single startup flow and no per-file prompt flood.
- Security boundaries remain strict for non-internal code paths.
