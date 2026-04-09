# Registry Runtime Track (Platform First)

This track is runtime/platform work that should be completed before linkhash app integration.

## Scope

- Runtime-level PHPX primitives and framework capabilities.
- No linkhash app endpoint/UI work in this file.

## Tasks

1. [x] Auth primitive quality pass (`php_modules/crypto`, `php_modules/jwt`, `php_modules/cookies`, `php_modules/auth`)
1. [x] JWT sign/verify/claim timing behavior smoke tests (`tests/phpx/foundation/jwt_*`)
1. [x] Cookies behavior smoke tests (`tests/phpx/foundation/cookies_*`)
1. [x] Add missing module docs for auth primitives under `docs/php/` or `docs/phpx/phpx-dx.md`

2. [x] Framework context provider model (server-first)
1. [x] per-request context store in runtime/framework layer
1. [x] provider composition at route/layout boundary
1. [x] `auth()` helper for current request auth state
1. [x] `requireAuth()` guard helper
1. [x] `requireScope(...)` guard helper
1. [x] `useContext(...)` helper for component render
1. [x] context re-evaluation on partial navigation
1. [x] minimal safe auth snapshot for hydration (no secrets)

3. [x] Runtime-facing CLI glue for registry workflows
1. [x] `deka publish` auth token plumbing (`LINKHASH_TOKEN`)
1. [x] `deka install` php ecosystem registry resolution path
1. [x] lockfile write path under shared `deka.lock` (`php` section)
1. [x] docs for `.env` token setup and scope behavior

4. [x] Runtime hardening items
1. [x] enforce PKCE + OAuth state + nonce checks in shared auth flow helpers
1. [x] runtime tests for auth failure modes and scope-denied paths

## Validation

1. [x] `cargo test -p php-rs --release`
1. [x] `node tests/phpx/testrunner.js tests/phpx/foundation`
1. [x] one release-build smoke run using `target/release/cli run <fixture>.phpx`
