# Privileged Framework Routing

## Problem

`component/router` is framework code, but today it discovers routes by scanning `app/` through userland filesystem APIs.

Under default-deny security, that creates the wrong trust boundary:

- fresh projects break unless userland code is granted broad read access
- framework mechanics are mixed with untrusted module permissions
- the default app experience depends on filesystem policy rather than a stable runtime contract

## Goal

Keep routing in userland, but move route discovery behind a privileged internal boundary owned by the runtime.

The router should consume route metadata, not raw filesystem access.

## Proposed Model

1. Runtime scans `app/` and `api/` once at startup.
2. Runtime stores the route manifest in memory.
3. Runtime exposes that manifest through a narrowly scoped internal contract.
4. `component/router` consumes that contract.
5. Userland code never receives blanket directory-read access just to make framework routing work.

## Constraints

- Keep the explicit `main.phpx` entry contract.
- Do not reintroduce hidden runtime routing as the default app model.
- The privileged layer must expose data, not generic filesystem power.
- The contract must be reusable by future frameworks, not hard-coded to `component/router` only.

## Tasks

- [x] Define a typed manifest contract in `runtime_core`.
- [ ] Implement runtime startup scanning for `app/` and `api/`.
- [ ] Cache the manifest in memory per handler/project root.
- [ ] Expose the manifest to PHPX through a privileged internal bridge.
- [ ] Update `component/router` to consume the manifest instead of scanning the filesystem.
- [ ] Add regression tests for root page, nested pages, layouts, and dynamic segments.
- [ ] Decide whether legacy runtime-owned directory routing can be removed entirely.

## First Step

The first implementation step is the data contract:

- define manifest structs in Rust
- define route normalization rules from app-relative paths
- cover the normalization with unit tests

This gives the runtime and the router a shared, typed boundary before behavior is wired to it.
