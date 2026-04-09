# Adwa Migration Plan

Status: active  
Owner: runtime + browser platform

## Objective

Replace legacy `adwa` naming with `adwa`, split platform code into its own repo, and move toward a Linux-like browser runtime mental model.

## Phase 1: Repository Split

- [ ] Create standalone repo at `~/Projects/deka/adwa`.
- [ ] Copy current platform source from `deka/adwa` into `adwa`.
- [ ] Set `adwa` as canonical source for browser runtime/platform work.
- [ ] Keep transitional compatibility from `deka` until integration is fully switched.

## Phase 2: Naming Migration

- [ ] Rename `adwa` identifiers to `adwa` across source/docs/scripts.
- [ ] Rename package/module names where needed (`adwa_*` -> `adwa_*`).
- [ ] Update runtime/demo wiring in `deka` to call into `adwa`.
- [ ] Preserve deterministic build/test scripts during rename.

## Phase 3: Linux-like Runtime Defaults

- [ ] Bootstrap a Linux-style base FS tree:
  - `/bin`, `/usr/bin`, `/etc`, `/home/<user>`, `/tmp`, `/var`, `/dev`, `/proc`
- [ ] Mount project workspace at `/workspace` and expose convenient home path.
- [ ] Set shell env defaults:
  - `HOME`, `USER`, `PWD`, `SHELL`, `PATH`
- [ ] Make command resolution PATH-first instead of ad-hoc module-only shortcuts.

## Phase 4: Parity + Cleanup

- [ ] Keep browser e2e and host bridge smokes green through migration.
- [ ] Finish server/browser command behavior parity checks.
- [ ] Remove old `adwa` naming and transitional compatibility code once stable.
