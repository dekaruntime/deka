# MVP Runtime Module Resolution Plan

Goal: make module resolution lockfile-driven across local + global `php_modules`, with deterministic behavior and no ADWA/UI magic.

  ## Rules
  - [x] 1. Resolver order is deterministic:
  1) local `<project>/php_modules`
  2) global `<PHPX_MODULE_ROOT>/php_modules`
  - [x] 2. Resolver is lockfile-driven (`deka.lock` is source of truth for php modules).
  - [x] 3. Runtime validates loaded module integrity against lock metadata.
  - [x] 4. Support shorthand and index resolution consistently:
  - `@/components/Card` -> `Card.phpx` or `Card/index.phpx`
  - package import equivalents in `php_modules`
  - [x] 5. Cache keys include lock hash/version to prevent stale loads.
  - [x] 6. Single global env var only: `PHPX_MODULE_ROOT` (remove legacy aliases).
- [ ] 7. Runtime/installer contract:
  - installer writes lock + bytes to correct tier (local/global)
  - runtime refuses drift from lock
  - [x] 8. Explicit actionable errors for:
  - missing lock
  - missing module
  - integrity mismatch
  - unresolved symbol/export

## Execution Plan

  ## Task 1: Baseline and Test Harness
  - [x] Locate active resolver/lock code paths in `crates/php-rs` and `crates/cli`.
  - [x] Add/extend integration tests for:
  - local-only
  - global-only
  - local-over-global precedence
  - missing lock/module failures
  - [x] Run tests.
  - [x] Commit: `test(runtime): add module resolution baseline coverage`

  ## Task 2: Add `PHPX_MODULE_ROOT` Resolver Tier
  - [x] Implement global tier support in runtime resolver.
  - [x] Define exact lock lookup behavior:
  - local project mode: local lock required
  - global/no-project mode: global lock required
  - [x] Remove/disable `DEKA_LOCK_ROOT` usage.
  - [x] Run tests.
- [x] Commit: `feat(runtime): add PHPX_MODULE_ROOT global module tier`

  ## Task 3: Lockfile-Driven Resolution
  - [x] Ensure module lookup first consults lock entries.
  - [x] Enforce that filesystem modules not in lock are rejected.
  - [x] Add diagnostics for lock miss with import path and expected entry.
  - [x] Run tests.
- [x] Commit: `feat(runtime): enforce lockfile-driven php module resolution`

  ## Task 4: Integrity Validation
  - [x] Extend lock schema/reader as needed for php module integrity fields.
  - [x] Validate loaded module bytes against lock hash/integrity.
  - [x] Fail fast on mismatch with clear remediation text.
  - [x] Run tests.
- [x] Commit: `feat(runtime): validate php module integrity against lock`

  ## Task 5: Import Shorthand + Index Semantics
  - [x] Normalize import resolver behavior for:
  - `Foo` -> `Foo.phpx`
  - `Foo` -> `Foo/index.phpx`
  - [x] Apply same rules for local aliases (`@/...`) and package paths.
  - [x] Add tests covering ambiguous/invalid cases.
  - [x] Run tests.
- [x] Commit: `fix(runtime): unify php module shorthand/index resolution`

  ## Task 6: Cache Correctness
  - [x] Include lock hash/version in compiled cache key.
  - [x] Invalidate on lock change or module hash change.
  - [x] Add regression tests for stale cache prevention.
  - [x] Run tests.
- [x] Commit: `fix(runtime): key module cache by lock identity`

## Task 7: Installer + Runtime Contract
- [x] Ensure installer writes lock entries with tier + integrity metadata.
- [x] Ensure installer places bytes under correct roots:
  - local project `php_modules`
  - global `PHPX_MODULE_ROOT/php_modules`
- [x] Add cross-check tests with runtime loader.
- [x] Run tests.
- [x] Commit: `feat(cli): align install output with runtime lock contract`

## Task 8: Error UX and Diagnostics
- [x] Standardize resolver errors using deka-validation style where applicable.
- [x] Include:
  - import location
  - attempted roots
  - lock status
  - next-step remediation
- [x] Run tests.
- [x] Commit: `chore(runtime): improve module resolution diagnostics`

## Task 9: ADWA/Process-Model Validation (No Magic)
- [x] Verify command execution path is binary/module-based only.
- [x] Verify runtime resolver is the same path used by ADWA-launched commands.
- [x] Add one integration test proving `ls`/`deka db` path uses runtime resolution.
- [x] Run tests.
- [x] Commit: `test(adwa): validate process-model command resolution via runtime`

## Task 10: Docs + Agent Notes
- [x] Update AGENTS.md:
  - local/global module behavior
  - `PHPX_MODULE_ROOT` usage
  - lockfile as source of truth
  - test+commit requirement per task
- [x] Update runtime/docs pages for module resolution contract.
- [x] Run doc checks if available.
- [x] Commit: `docs: document php module resolution contract for MVP`

## Definition of Done
- [x] All tasks checked.
- [x] All relevant tests passing.
- [x] One commit per task.
- [x] No legacy root env behavior.
- [x] Deterministic lock-driven local/global resolution confirmed.
