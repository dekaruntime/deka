# Linkhash CLI DX Aliases + Namespacing

## Goals
- Add auth-first command UX (`deka login`, `deka auth login`).
- Make package UX npm-like (`deka add`, `deka i`, `deka pkg publish`, `deka publish`).
- Enforce scoped package names for PHP packages.
- Support stdlib shorthand aliases (`json`, `jwt`) -> `@deka/*`.

## Completed
- [x] Added `auth` command with subcommands: `login`, `logout`, `whoami`.
- [x] Added top-level aliases: `login`, `logout`, `whoami`.
- [x] Added local auth profile persistence at `~/.config/deka/auth.json`.
- [x] Added interactive `deka login` prompt flow when flags are omitted.
- [x] Added `pkg` command with subcommands `install` and `publish`.
- [x] Registered top-level `publish` + `pkg publish`.
- [x] Added top-level `add` and `i` commands (PHP install shorthand).
- [x] Enforced PHP package scoping (`@scope/name`), with shorthand mapping:
  - `json` -> `@deka/json`
  - `jwt` -> `@deka/jwt`
- [x] Updated `publish` command to Linkhash-native endpoint `/api/packages/publish`.
- [x] `publish` now uses auth profile fallback for token/registry.

## Validation
- `cargo check -p cli -p pm` (pass)
- `cargo build --release -p cli` (pass)
- Smoke:
  - `deka login --username sam --token ...` => saved as `@sam`
  - `deka login` (interactive stdin) => prompts for username/token/registry
  - `deka whoami` / `deka auth whoami` => shows saved profile
  - `deka add foo` => rejects unscoped non-stdlib package
  - `deka pkg publish` / `deka publish` both wired
