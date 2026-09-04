# Platform Contracts (MVP)

This document defines the reboot host boundary for Deka runtime portability.

## Goal

Keep runtime core host-agnostic by routing all host access through a `platform` contract crate.

## Contracts

- `Fs`
  - File reads/writes and directory creation.
  - Existence checks and current working directory.
- `Env`
  - Environment variable lookup and enumeration.
- `Io`
  - Standard output and error output.
- `Process`
  - CLI args, process id, and termination.
- `Time`
  - Current unix time in milliseconds and sleep.
- `Random`
  - Cryptographic/random byte fill.
- `Net`
  - Controlled network fetch surface.
- `Ports`
  - Port reservation and release.

## Platform Implementations

- `platform_server`
  - Uses server host APIs.
  - Owns all direct Deno/OS bindings.
- `platform_browser`
  - Uses ADWA/browser-safe primitives.
  - Applies strict URL/module restrictions.

## Boundary Rule

Runtime core code must not call host APIs directly. It only receives a `Platform` implementation.
